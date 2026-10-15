//! Mac apps that update through Sparkle, outside Homebrew and the App Store.
//!
//! Each app found in `/Applications` or `~/Applications` that names a Sparkle
//! feed is checked against that feed. Apps the Mac App Store manages
//! (`Contents/_MASReceipt`) and apps a Homebrew cask installed are skipped:
//! those have their own updater, and two tools updating one app is how an app
//! ends up half-replaced.
//!
//! ## What an update must pass before it replaces an app
//!
//! 1. The release is the one the scan showed, and the app is not running.
//! 2. The archive is HTTPS-downloaded and — when the installed app declares
//!    an `SUPublicEDKey` — carries a valid Ed25519 signature from that key.
//!    The key comes from the app already on the disk, exactly as Sparkle
//!    itself checks it.
//! 3. The new bundle has the same bundle identifier, a valid code signature,
//!    the **same Team ID** as the installed app, and Gatekeeper accepts it
//!    (`spctl`), which also covers notarisation.
//! 4. The swap is reversible until it has succeeded: the old app is renamed
//!    aside, the new one moved in, and only then is the old one deleted. If
//!    the move fails the old app is put back.
//!
//! Only `.zip` and `.dmg` archives are handled. Installer packages and delta
//! updates need Sparkle's own machinery and are left to the app.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// What Odysync needs from an app's `Info.plist`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacApp {
    pub path: PathBuf,
    pub bundle_id: String,
    pub name: String,
    pub short_version: Option<String>,
    pub build: String,
    pub feed_url: Option<String>,
    pub public_ed_key: Option<String>,
}

#[derive(Deserialize)]
struct InfoPlist {
    #[serde(rename = "CFBundleIdentifier")]
    bundle_id: Option<String>,
    #[serde(rename = "CFBundleDisplayName")]
    display_name: Option<String>,
    #[serde(rename = "CFBundleName")]
    bundle_name: Option<String>,
    #[serde(rename = "CFBundleShortVersionString")]
    short_version: Option<String>,
    #[serde(rename = "CFBundleVersion")]
    build: Option<String>,
    #[serde(rename = "SUFeedURL")]
    feed_url: Option<String>,
    #[serde(rename = "SUPublicEDKey")]
    public_ed_key: Option<String>,
}

/// Read an app's `Info.plist` (XML or binary) into a [`MacApp`].
pub fn parse_info_plist(path: &Path, bytes: &[u8]) -> Option<MacApp> {
    let info: InfoPlist = plist::from_bytes(bytes).ok()?;
    let trim = |s: Option<String>| s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let bundle_id = trim(info.bundle_id)?;
    let build = trim(info.build)?;
    let fallback = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| bundle_id.clone());
    Some(MacApp {
        path: path.to_path_buf(),
        name: trim(info.display_name)
            .or(trim(info.bundle_name))
            .unwrap_or(fallback),
        bundle_id,
        short_version: trim(info.short_version),
        build,
        // Sparkle only ever fetches its feed over HTTPS in current versions;
        // an http:// feed is an app Odysync will not second-guess.
        feed_url: trim(info.feed_url).filter(|u| u.starts_with("https://")),
        public_ed_key: trim(info.public_ed_key),
    })
}

/// `TeamIdentifier=ABCDE12345` from `codesign -dv` (which writes to stderr).
/// `not set` — an ad-hoc or unsigned app — is no Team ID at all.
pub fn parse_team_id(codesign_output: &str) -> Option<String> {
    codesign_output.lines().find_map(|l| {
        let v = l.trim().strip_prefix("TeamIdentifier=")?.trim();
        (!v.is_empty() && v != "not set").then(|| v.to_string())
    })
}

/// App names a Homebrew cask installed, from `brew info --cask --json=v2`.
pub fn parse_brew_cask_apps(json: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for cask in value
        .get("casks")
        .and_then(|c| c.as_array())
        .into_iter()
        .flatten()
    {
        for artifact in cask
            .get("artifacts")
            .and_then(|a| a.as_array())
            .into_iter()
            .flatten()
        {
            for app in artifact
                .get("app")
                .and_then(|a| a.as_array())
                .into_iter()
                .flatten()
            {
                if let Some(name) = app.as_str() {
                    // `"Foo.app"`, or a renamed target object; the plain name
                    // is the common case and the one that matters here.
                    out.push(name.to_string());
                }
            }
        }
    }
    out
}

/// Whether any running executable lives inside `app`.
pub fn is_running(app: &Path, ps_output: &str) -> bool {
    let prefix = format!(
        "{}/Contents/",
        app.display().to_string().trim_end_matches('/')
    );
    ps_output.lines().any(|l| l.trim().starts_with(&prefix))
}

/// Replace `installed` with `new`, keeping the old copy until the new one is
/// in place.
///
/// Both must be on the same volume for the renames; `copy` is the fallback
/// used when moving `new` in fails (on macOS, `ditto`). On any failure the old
/// app is restored and the error returned.
pub fn swap_bundle(
    installed: &Path,
    new: &Path,
    copy: impl Fn(&Path, &Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let parent = installed
        .parent()
        .ok_or_else(|| std::io::Error::other("the app has no parent folder"))?;
    let name = installed
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "app".into());
    let backup = parent.join(format!(".odysync-previous-{}-{name}", std::process::id()));

    std::fs::rename(installed, &backup)?;
    let moved = std::fs::rename(new, installed).or_else(|_| copy(new, installed));
    match moved {
        Ok(()) => {
            if let Err(e) = std::fs::remove_dir_all(&backup) {
                tracing::warn!(path = %backup.display(), error = %e, "could not remove the previous app copy");
            }
            Ok(())
        }
        Err(e) => {
            // Put the old app back. A partial copy is removed first so the
            // rename has somewhere to go.
            let _ = std::fs::remove_dir_all(installed);
            std::fs::rename(&backup, installed)?;
            Err(e)
        }
    }
}

#[cfg(target_os = "macos")]
pub mod imp {
    use std::time::Duration;

    use odysync_core::error::{Error, Result};
    use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
    use odysync_core::proc;
    use odysync_core::version::Version;

    use super::*;
    use crate::direct::{http, sparkle};

    pub const NATIVE_PREFIX: &str = "app:";
    const TOOL_TIMEOUT: Duration = Duration::from_secs(120);

    fn app_dirs() -> Vec<PathBuf> {
        let mut dirs = vec![PathBuf::from("/Applications")];
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(home).join("Applications"));
        }
        dirs
    }

    pub fn installed_apps() -> Vec<MacApp> {
        let mut out = Vec::new();
        for dir in app_dirs() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("app") {
                    continue;
                }
                // Mac App Store apps update through the App Store.
                if path.join("Contents").join("_MASReceipt").exists() {
                    continue;
                }
                let Ok(bytes) = std::fs::read(path.join("Contents").join("Info.plist")) else {
                    continue;
                };
                if let Some(app) = parse_info_plist(&path, &bytes) {
                    out.push(app);
                }
            }
        }
        out
    }

    async fn homebrew_apps() -> Vec<String> {
        match proc::run(
            "brew",
            &["info", "--cask", "--installed", "--json=v2"],
            TOOL_TIMEOUT,
        )
        .await
        {
            Ok(out) if out.success() => parse_brew_cask_apps(&out.stdout),
            _ => Vec::new(),
        }
    }

    async fn os_version() -> String {
        proc::run("sw_vers", &["-productVersion"], TOOL_TIMEOUT)
            .await
            .map(|o| o.stdout.trim().to_string())
            .unwrap_or_default()
    }

    /// Apps with a Sparkle feed, minus the ones Homebrew owns.
    async fn sparkle_apps() -> Vec<MacApp> {
        let brew = homebrew_apps().await;
        tokio::task::spawn_blocking(installed_apps)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.feed_url.is_some())
            .filter(|a| {
                let file = a
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                !brew.iter().any(|b| b.eq_ignore_ascii_case(&file))
            })
            .collect()
    }

    async fn latest_for(
        client: &reqwest::Client,
        app: &MacApp,
        os: &str,
    ) -> Result<Option<sparkle::AppcastRelease>> {
        let Some(feed) = &app.feed_url else {
            return Ok(None);
        };
        let xml = http::get_text(client, feed).await?;
        let releases = sparkle::parse_appcast(&xml);
        Ok(sparkle::newest_compatible(&releases, os).filter(|r| {
            let lower = r.url.to_ascii_lowercase();
            lower.ends_with(".zip") || lower.ends_with(".dmg")
        }))
    }

    pub async fn scan(client: &reqwest::Client) -> Vec<UpdateCandidate> {
        let os = os_version().await;
        let apps = sparkle_apps().await;
        let lookups = apps
            .iter()
            .map(|app| async { (app, latest_for(client, app, &os).await) });
        let mut out = Vec::new();
        for (app, result) in futures::future::join_all(lookups).await {
            match result {
                Ok(Some(release)) => {
                    let (installed, available) = sparkle::comparable_versions(
                        app.short_version.as_deref(),
                        &app.build,
                        &release,
                    );
                    out.push(UpdateCandidate {
                        id: PackageId::new(
                            BackendKind::Direct,
                            format!("{NATIVE_PREFIX}{}", app.bundle_id),
                        )
                        .with_alias(&app.name),
                        name: app.name.clone(),
                        installed: Version::parse(&installed),
                        available: Version::parse(&available),
                        size_bytes: release.length,
                        expected_sha256: None,
                    });
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(app = %app.bundle_id, error = %e, "could not read the app's update feed")
                }
            }
        }
        out
    }

    fn find(bundle_id: &str) -> Option<MacApp> {
        installed_apps()
            .into_iter()
            .find(|a| a.bundle_id == bundle_id)
    }

    pub async fn installed_version(candidate: &UpdateCandidate) -> Option<String> {
        let bundle_id = candidate.id.native.strip_prefix(NATIVE_PREFIX)?.to_string();
        let app = tokio::task::spawn_blocking(move || find(&bundle_id))
            .await
            .ok()??;
        let wanted = candidate.available.raw().trim();
        if app.short_version.as_deref() == Some(wanted) {
            app.short_version
        } else if app.build == wanted {
            Some(app.build)
        } else {
            app.short_version.or(Some(app.build))
        }
    }

    async fn team_id(app: &Path) -> Option<String> {
        let path = app.display().to_string();
        let out = proc::run("codesign", &["-dv", "--verbose=2", &path], TOOL_TIMEOUT)
            .await
            .ok()?;
        parse_team_id(&format!("{}\n{}", out.stdout, out.stderr))
    }

    fn refuse(app: &str, detail: impl Into<String>) -> Error {
        Error::Verification {
            package: app.to_string(),
            detail: detail.into(),
        }
    }

    /// Extract `archive` into `dest` and return the `.app` inside it.
    async fn extract(archive: &Path, dest: &Path, expected_name: &str) -> Result<PathBuf> {
        let lower = archive.display().to_string().to_ascii_lowercase();
        let archive_s = archive.display().to_string();
        let dest_s = dest.display().to_string();
        if lower.ends_with(".zip") {
            let out = proc::run("ditto", &["-x", "-k", &archive_s, &dest_s], TOOL_TIMEOUT).await?;
            if !out.success() {
                return Err(Error::CommandFailed {
                    command: "ditto -x -k".into(),
                    code: out.code,
                    stderr: out.stderr,
                });
            }
        } else if lower.ends_with(".dmg") {
            let mount = dest.join(".mount");
            std::fs::create_dir_all(&mount)?;
            let mount_s = mount.display().to_string();
            let out = proc::run(
                "hdiutil",
                &[
                    "attach",
                    "-nobrowse",
                    "-readonly",
                    "-noautoopen",
                    "-mountpoint",
                    &mount_s,
                    &archive_s,
                ],
                TOOL_TIMEOUT,
            )
            .await?;
            if !out.success() {
                return Err(Error::CommandFailed {
                    command: "hdiutil attach".into(),
                    code: out.code,
                    stderr: out.stderr,
                });
            }
            let copied = async {
                let app = std::fs::read_dir(&mount)?
                    .flatten()
                    .map(|e| e.path())
                    .find(|p| p.extension().and_then(|e| e.to_str()) == Some("app"))
                    .ok_or_else(|| Error::parse("disk image", "no app inside"))?;
                let target = dest.join(app.file_name().unwrap_or_default());
                let out = proc::run(
                    "ditto",
                    &[app.display().to_string(), target.display().to_string()],
                    TOOL_TIMEOUT,
                )
                .await?;
                if out.success() {
                    Ok(())
                } else {
                    Err(Error::CommandFailed {
                        command: "ditto".into(),
                        code: out.code,
                        stderr: out.stderr,
                    })
                }
            }
            .await;
            let _ = proc::run("hdiutil", &["detach", &mount_s, "-force"], TOOL_TIMEOUT).await;
            let _ = std::fs::remove_dir(&mount);
            copied?;
        } else {
            return Err(Error::unavailable(
                "direct",
                "only .zip and .dmg updates are handled; update this app from inside it",
            ));
        }

        let apps: Vec<PathBuf> = std::fs::read_dir(dest)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("app"))
            .collect();
        apps.iter()
            .find(|p| p.file_name().and_then(|n| n.to_str()) == Some(expected_name))
            .or_else(|| (apps.len() == 1).then(|| &apps[0]))
            .cloned()
            .ok_or_else(|| Error::parse("update archive", "could not find the app inside"))
    }

    pub async fn apply(candidate: &UpdateCandidate, client: &reqwest::Client) -> Result<()> {
        let bundle_id = candidate
            .id
            .native
            .strip_prefix(NATIVE_PREFIX)
            .ok_or_else(|| Error::invalid_package_id(&candidate.id.native, "not a Mac app id"))?
            .to_string();
        let lookup = bundle_id.clone();
        let app = tokio::task::spawn_blocking(move || find(&lookup))
            .await
            .ok()
            .flatten()
            .ok_or_else(|| {
                Error::unavailable("direct", format!("{bundle_id} is no longer installed"))
            })?;

        // 1. The approved release, and the app closed.
        let os = os_version().await;
        let release = latest_for(client, &app, &os)
            .await?
            .ok_or_else(|| Error::unavailable("direct", "the feed no longer offers an update"))?;
        let (_, available) =
            sparkle::comparable_versions(app.short_version.as_deref(), &app.build, &release);
        if available.trim() != candidate.available.raw().trim() {
            return Err(refuse(
                &app.name,
                format!(
                    "the feed now offers {available} instead of the approved {}; scan again",
                    candidate.available.raw()
                ),
            ));
        }
        let ps = proc::run("ps", &["-axo", "comm="], TOOL_TIMEOUT).await?;
        if is_running(&app.path, &ps.stdout) {
            return Err(Error::unavailable(
                "direct",
                format!("{} is running; quit it and try again", app.name),
            ));
        }
        let installed_team = team_id(&app.path).await.ok_or_else(|| {
            refuse(
                &app.name,
                "the installed app has no Team ID, so an update cannot be matched to its developer",
            )
        })?;

        let work = http::download_dir()?.join(format!(
            "mac-{}-{}",
            bundle_id.replace(|c: char| !c.is_ascii_alphanumeric(), "-"),
            chrono::Utc::now().timestamp_millis()
        ));
        std::fs::create_dir_all(&work)?;
        let archive = work.join(http::file_name_for(&release.url, "update.zip"));

        let result = async {
            // 2. Download, then the app's own EdDSA key.
            http::download(client, &release.url, &archive, |_, _| {}).await?;
            if let Some(key) = &app.public_ed_key {
                let signature = release.ed_signature.as_deref().ok_or_else(|| {
                    refuse(
                        &app.name,
                        "the app requires signed updates but the feed has no signature",
                    )
                })?;
                let bytes = tokio::fs::read(&archive).await?;
                if !sparkle::verify_ed_signature(key, signature, &bytes) {
                    return Err(refuse(
                        &app.name,
                        "the update's EdDSA signature does not match the app's key",
                    ));
                }
            }

            // 3. Same app, same developer, accepted by Gatekeeper.
            let extracted = work.join("extracted");
            std::fs::create_dir_all(&extracted)?;
            let expected_name = app
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let new_app = extract(&archive, &extracted, &expected_name).await?;
            let info = std::fs::read(new_app.join("Contents").join("Info.plist"))?;
            let new_info = parse_info_plist(&new_app, &info)
                .ok_or_else(|| refuse(&app.name, "the update has no readable Info.plist"))?;
            if new_info.bundle_id != app.bundle_id {
                return Err(refuse(
                    &app.name,
                    format!("the update is a different app ({})", new_info.bundle_id),
                ));
            }
            let new_s = new_app.display().to_string();
            let verify = proc::run(
                "codesign",
                &["--verify", "--deep", "--strict", &new_s],
                TOOL_TIMEOUT,
            )
            .await?;
            if !verify.success() {
                return Err(refuse(
                    &app.name,
                    format!("code signature invalid: {}", verify.stderr.trim()),
                ));
            }
            match team_id(&new_app).await {
                Some(t) if t == installed_team => {}
                other => return Err(refuse(
                    &app.name,
                    format!(
                        "signed by Team ID {other:?}, but the installed app is {installed_team}"
                    ),
                )),
            }
            let gatekeeper = proc::run(
                "spctl",
                &["--assess", "--type", "execute", &new_s],
                TOOL_TIMEOUT,
            )
            .await?;
            if !gatekeeper.success() {
                return Err(refuse(&app.name, "Gatekeeper does not accept the update"));
            }

            // 4. Reversible swap.
            let installed = app.path.clone();
            tokio::task::spawn_blocking(move || {
                swap_bundle(&installed, &new_app, |from, to| {
                    let status = std::process::Command::new("ditto")
                        .arg(from)
                        .arg(to)
                        .status()?;
                    if status.success() {
                        Ok(())
                    } else {
                        Err(std::io::Error::other("ditto failed"))
                    }
                })
            })
            .await
            .map_err(|e| Error::Io(std::io::Error::other(e)))??;
            Ok(())
        }
        .await;

        let _ = std::fs::remove_dir_all(&work);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>com.knollsoft.Rectangle</string>
  <key>CFBundleName</key><string>Rectangle</string>
  <key>CFBundleShortVersionString</key><string>2.0.2</string>
  <key>CFBundleVersion</key><string>109</string>
  <key>SUFeedURL</key><string>https://rectangleapp.com/downloads/updates.xml</string>
  <key>SUPublicEDKey</key><string>lpt9M3PhocbZ3MZiLH+crEqRfU11kfoNzGxSqiEIdvM=</string>
</dict></plist>"#;

    #[test]
    fn an_info_plist_yields_the_sparkle_settings() {
        let app = parse_info_plist(Path::new("/Applications/Rectangle.app"), INFO.as_bytes())
            .expect("parses");
        assert_eq!(app.bundle_id, "com.knollsoft.Rectangle");
        assert_eq!(app.name, "Rectangle");
        assert_eq!(app.short_version.as_deref(), Some("2.0.2"));
        assert_eq!(app.build, "109");
        assert_eq!(
            app.feed_url.as_deref(),
            Some("https://rectangleapp.com/downloads/updates.xml")
        );
        assert!(app.public_ed_key.is_some());
    }

    #[test]
    fn a_binary_plist_parses_too() {
        let value: plist::Value = plist::from_bytes(INFO.as_bytes()).unwrap();
        let mut binary = Vec::new();
        plist::to_writer_binary(&mut binary, &value).unwrap();
        let app = parse_info_plist(Path::new("/Applications/Rectangle.app"), &binary).unwrap();
        assert_eq!(app.build, "109");
    }

    #[test]
    fn an_http_feed_is_not_followed() {
        let insecure = INFO.replace("https://rectangleapp.com", "http://rectangleapp.com");
        let app = parse_info_plist(Path::new("/Applications/R.app"), insecure.as_bytes()).unwrap();
        assert_eq!(app.feed_url, None);
    }

    #[test]
    fn the_team_id_is_read_and_an_unset_one_is_none() {
        let out = "Executable=/Applications/Rectangle.app/Contents/MacOS/Rectangle\n\
                   Identifier=com.knollsoft.Rectangle\n\
                   Authority=Developer ID Application: Ryan Hanson (XSYZ3E4B7D)\n\
                   TeamIdentifier=XSYZ3E4B7D\n";
        assert_eq!(parse_team_id(out).as_deref(), Some("XSYZ3E4B7D"));
        assert_eq!(parse_team_id("TeamIdentifier=not set\n"), None);
        assert_eq!(parse_team_id(""), None);
    }

    #[test]
    fn apps_installed_by_homebrew_casks_are_recognised() {
        let json = r#"{"formulae":[],"casks":[
            {"token":"rectangle","artifacts":[{"app":["Rectangle.app"]},{"zap":[{"trash":["~/x"]}]}]},
            {"token":"font-x","artifacts":[{"font":["x.ttf"]}]}
        ]}"#;
        assert_eq!(
            parse_brew_cask_apps(json),
            vec!["Rectangle.app".to_string()]
        );
        assert!(parse_brew_cask_apps("garbage").is_empty());
    }

    #[test]
    fn a_running_app_is_detected_by_its_executable_path() {
        let ps = "/sbin/launchd\n/Applications/Rectangle.app/Contents/MacOS/Rectangle\n";
        assert!(is_running(Path::new("/Applications/Rectangle.app"), ps));
        assert!(!is_running(Path::new("/Applications/Rect.app"), ps));
        assert!(!is_running(Path::new("/Applications/Maccy.app"), ps));
    }

    fn bundle(dir: &Path, name: &str, marker: &str) -> PathBuf {
        let app = dir.join(name);
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        std::fs::write(app.join("Contents").join("marker"), marker).unwrap();
        app
    }

    #[test]
    fn a_swap_replaces_the_app_and_removes_the_old_copy() {
        let dir = tempfile::tempdir().unwrap();
        let installed = bundle(dir.path(), "App.app", "old");
        let new = bundle(&dir.path().join("staging"), "App.app", "new");

        swap_bundle(&installed, &new, |_, _| panic!("rename should have worked")).unwrap();

        let marker = std::fs::read_to_string(installed.join("Contents").join("marker")).unwrap();
        assert_eq!(marker, "new");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".odysync-previous")
            })
            .collect();
        assert!(leftovers.is_empty(), "the previous copy was left behind");
    }

    #[test]
    fn a_failed_swap_puts_the_old_app_back() {
        let dir = tempfile::tempdir().unwrap();
        let installed = bundle(dir.path(), "App.app", "old");
        // A source that does not exist makes the rename fail; the copy
        // fallback fails too.
        let missing = dir.path().join("nowhere").join("App.app");

        let err = swap_bundle(&installed, &missing, |_, _| {
            Err(std::io::Error::other("copy failed"))
        });
        assert!(err.is_err());
        let marker = std::fs::read_to_string(installed.join("Contents").join("marker")).unwrap();
        assert_eq!(
            marker, "old",
            "the original app must survive a failed update"
        );
    }
}
