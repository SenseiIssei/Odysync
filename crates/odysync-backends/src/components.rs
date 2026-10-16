//! Runtime components games and desktop apps depend on.
//!
//! The part of Driver Booster's "Game Components" that is genuinely useful,
//! done with Microsoft's own installers only:
//!
//! * **Visual C++ Redistributable (v14)** — the runtime almost every Windows
//!   game links against. Installed or repaired from Microsoft's permanent
//!   `aka.ms` links. The installer refuses to downgrade on its own, so
//!   "install the latest" is safe to offer even when the installed version is
//!   already current.
//! * **DirectX End-User Runtime** — the legacy D3DX9/10/11, XAudio 2.7 and
//!   XInput 1.3 libraries older games still load. Windows' own DirectX is
//!   untouched by it.
//! * **.NET Desktop Runtime / .NET Runtime** — the latest *patch* of each
//!   major line already installed, from Microsoft's release metadata, with
//!   the SHA-512 Microsoft publishes. A new major line is never installed on
//!   the user's behalf: apps target a major version, not "the newest".
//!
//! What Driver Booster also offers and this deliberately does not: dropping
//! individual DLLs into `System32` from a "missing DLL" database. That is how
//! DLL-download sites work, and it is how machines end up with mismatched,
//! unsigned system libraries.

use serde::{Deserialize, Serialize};

/// What Odysync knows about one component on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentStatus {
    /// Stable id for the CLI and the GUI, e.g. `vc-redist-x64`.
    pub id: String,
    pub name: String,
    /// Installed version, or `None` when the component is missing.
    pub installed: Option<String>,
    /// Latest version Microsoft offers, when that is knowable without
    /// downloading the installer.
    pub latest: Option<String>,
    pub state: ComponentState,
    /// Whether Odysync can install or repair it from here.
    pub installable: bool,
    /// One sentence of context for the user.
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentState {
    Missing,
    Installed,
    Outdated,
    /// Installed, but Microsoft no longer ships security fixes for it.
    OutOfSupport,
}

// ── .NET release metadata (pure) ───────────────────────────────────────────

/// One major line from `releases-index.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotnetChannel {
    /// `8.0`, `10.0`.
    pub channel: String,
    pub latest_runtime: String,
    /// `active`, `maintenance`, `eol`, `preview`, `go-live`.
    pub support_phase: String,
    pub releases_json: String,
}

pub fn parse_dotnet_index(json: &serde_json::Value) -> Vec<DotnetChannel> {
    json.get("releases-index")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let text = |k: &str| r.get(k).and_then(|v| v.as_str()).map(str::to_string);
            Some(DotnetChannel {
                channel: text("channel-version")?,
                latest_runtime: text("latest-runtime")?,
                support_phase: text("support-phase").unwrap_or_default(),
                releases_json: text("releases.json")?,
            })
        })
        .collect()
}

/// The `major.minor` line a version belongs to: `8.0.29` → `8.0`.
pub fn dotnet_channel_of(version: &str) -> Option<String> {
    let mut parts = version.split('.');
    let major = parts.next()?;
    let minor = parts.next()?;
    (major.bytes().all(|b| b.is_ascii_digit()) && minor.bytes().all(|b| b.is_ascii_digit()))
        .then(|| format!("{major}.{minor}"))
}

/// The installer for `product` (`windowsdesktop` or `runtime`) and `rid`
/// (`win-x64`) in a channel's `releases.json`, for its newest release.
///
/// Returns `(version, url, sha512)`. Only `.exe` installers hosted by
/// Microsoft over HTTPS are accepted.
pub fn dotnet_installer(
    releases: &serde_json::Value,
    product: &str,
    rid: &str,
) -> Option<(String, String, String)> {
    let release = releases.get("releases")?.as_array()?.first()?;
    let block = release.get(product)?;
    let version = block.get("version")?.as_str()?.to_string();
    let file = block.get("files")?.as_array()?.iter().find(|f| {
        f.get("rid").and_then(|v| v.as_str()) == Some(rid)
            && f.get("name")
                .and_then(|v| v.as_str())
                .is_some_and(|n| n.ends_with(".exe"))
    })?;
    let url = file.get("url")?.as_str()?.to_string();
    let hash = file.get("hash")?.as_str()?.to_ascii_lowercase();
    let microsoft = url.starts_with("https://builds.dotnet.microsoft.com/")
        || url.starts_with("https://download.visualstudio.microsoft.com/");
    (microsoft && hash.len() == 128 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        .then_some((version, url, hash))
}

/// `v14.51.36247.00` → `14.51.36247.0`, the form the VC++ installer reports.
pub fn normalize_vc_version(raw: &str) -> String {
    let trimmed = raw.trim().trim_start_matches(['v', 'V']);
    trimmed
        .split('.')
        .map(|p| p.trim_start_matches('0'))
        .map(|p| if p.is_empty() { "0" } else { p })
        .collect::<Vec<_>>()
        .join(".")
}

/// The newest version directory name in a .NET `shared/<framework>` folder.
pub fn newest_per_channel(versions: &[String]) -> Vec<String> {
    use odysync_core::version::Version;
    let mut best: Vec<String> = Vec::new();
    for v in versions {
        // Previews never count as "the installed line" for patching.
        if v.contains('-') {
            continue;
        }
        let Some(channel) = dotnet_channel_of(v) else {
            continue;
        };
        match best
            .iter_mut()
            .find(|b| dotnet_channel_of(b).as_deref() == Some(channel.as_str()))
        {
            Some(existing) => {
                if Version::parse(existing).compare(&Version::parse(v))
                    == Some(std::cmp::Ordering::Less)
                {
                    *existing = v.clone();
                }
            }
            None => best.push(v.clone()),
        }
    }
    best.sort();
    best
}

// ── Windows ─────────────────────────────────────────────────────────────────

#[cfg(windows)]
mod windows_impl {
    use std::path::PathBuf;
    use std::time::Duration;

    use odysync_core::error::{Error, Result};
    use odysync_core::version::Version;

    use super::*;
    use crate::direct::http;
    use crate::direct::windows_apply::{ExpectedDigest, lock_and_verify};

    const MICROSOFT: &str = "Microsoft Corporation";
    const INSTALL_TIMEOUT: Duration = Duration::from_secs(20 * 60);
    const DOTNET_INDEX: &str =
        "https://builds.dotnet.microsoft.com/dotnet/release-metadata/releases-index.json";
    const DIRECTX_WEB_SETUP: &str = "https://download.microsoft.com/download/1/7/1/1718ccc4-6315-4d8e-9543-8e28a4e18c4c/dxwebsetup.exe";

    fn vc_url(arch: &str) -> String {
        format!("https://aka.ms/vc14/vc_redist.{arch}.exe")
    }

    /// Installed VC++ v14 runtime for `arch`, from the key the installer writes.
    fn vc_installed(arch: &str) -> Option<String> {
        use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY};
        let hklm = winreg::RegKey::predef(HKEY_LOCAL_MACHINE);
        // The x86 runtime registers in the 32-bit view on 64-bit Windows.
        let flags = if arch == "x86" {
            KEY_READ | KEY_WOW64_32KEY
        } else {
            KEY_READ
        };
        let key = hklm
            .open_subkey_with_flags(
                format!(r"SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\{arch}"),
                flags,
            )
            .ok()?;
        if key.get_value::<u32, _>("Installed").ok()? != 1 {
            return None;
        }
        key.get_value::<String, _>("Version")
            .ok()
            .map(|v| normalize_vc_version(&v))
    }

    fn system_dir(name: &str) -> PathBuf {
        let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        PathBuf::from(windir).join(name)
    }

    fn dotnet_shared(framework: &str) -> Vec<String> {
        let base = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
        let dir = PathBuf::from(base)
            .join("dotnet")
            .join("shared")
            .join(framework);
        let versions: Vec<String> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        newest_per_channel(&versions)
    }

    fn rid() -> &'static str {
        match crate::direct::inventory::machine_arch() {
            "arm64" => "win-arm64",
            "x86" => "win-x86",
            _ => "win-x64",
        }
    }

    pub async fn status() -> Vec<ComponentStatus> {
        let mut out = Vec::new();

        let machine = crate::direct::inventory::machine_arch();
        let mut vc_arches = vec!["x64", "x86"];
        if machine == "arm64" {
            vc_arches.insert(0, "arm64");
        }
        for arch in vc_arches {
            let installed = vc_installed(arch);
            out.push(ComponentStatus {
                id: format!("vc-redist-{arch}"),
                name: format!("Visual C++ Redistributable ({arch})"),
                state: if installed.is_some() {
                    ComponentState::Installed
                } else {
                    ComponentState::Missing
                },
                installed,
                latest: None,
                installable: true,
                note: "Installing fetches Microsoft's latest v14 runtime; it never \
                       replaces a newer one."
                    .into(),
            });
        }

        let d3dx = system_dir("System32").join("d3dx9_43.dll").exists();
        out.push(ComponentStatus {
            id: "directx-legacy".into(),
            name: "DirectX End-User Runtime (legacy D3DX, XInput 1.3)".into(),
            installed: d3dx.then(|| "June 2010".into()),
            latest: Some("June 2010".into()),
            state: if d3dx {
                ComponentState::Installed
            } else {
                ComponentState::Missing
            },
            installable: true,
            note: "Only older games need these libraries. Windows' own DirectX is not \
                   changed."
                .into(),
        });

        // .NET: the installed lines, compared with Microsoft's metadata.
        let channels = match http::client(None, Duration::from_secs(30)) {
            Ok(client) => match http::get_json(&client, DOTNET_INDEX).await {
                Ok(json) => parse_dotnet_index(&json),
                Err(e) => {
                    tracing::warn!(error = %e, "could not read .NET release metadata");
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        let desktop = dotnet_shared("Microsoft.WindowsDesktop.App");
        let core = dotnet_shared("Microsoft.NETCore.App");
        let none: Vec<String> = Vec::new();
        for (product, label, installed_lines, skip) in [
            ("windowsdesktop", ".NET Desktop Runtime", &desktop, &none),
            // A line that has the desktop runtime is patched through it; the
            // base runtime then only needs offering where it stands alone.
            ("runtime", ".NET Runtime", &core, &desktop),
        ] {
            for version in installed_lines {
                let Some(line) = dotnet_channel_of(version) else {
                    continue;
                };
                if skip
                    .iter()
                    .any(|d| dotnet_channel_of(d).as_deref() == Some(line.as_str()))
                {
                    continue;
                }
                let channel = channels.iter().find(|c| c.channel == line);
                let latest = channel.map(|c| c.latest_runtime.clone());
                let eol = channel.is_some_and(|c| c.support_phase == "eol");
                let outdated = latest.as_ref().is_some_and(|l| {
                    Version::parse(version).compare(&Version::parse(l))
                        == Some(std::cmp::Ordering::Less)
                });
                let state = if eol {
                    ComponentState::OutOfSupport
                } else if outdated {
                    ComponentState::Outdated
                } else {
                    ComponentState::Installed
                };
                out.push(ComponentStatus {
                    id: format!("dotnet-{product}-{line}"),
                    name: format!("{label} {line}"),
                    installed: Some(version.clone()),
                    latest,
                    state,
                    installable: !eol && outdated && channel.is_some(),
                    note: if eol {
                        format!(
                            "Microsoft no longer fixes security issues in .NET {line}. \
                             Apps that need it keep working; consider asking their \
                             developers about a newer version."
                        )
                    } else {
                        format!("Patches stay within .NET {line}; Odysync never changes the major version.")
                    },
                });
            }
        }

        out
    }

    /// Install or repair one component. Needs administrator rights: every
    /// one of these installers writes machine-wide.
    pub async fn install(id: &str) -> Result<String> {
        if !odysync_core::platform::is_elevated() {
            return Err(Error::unavailable(
                "components",
                "installing runtime components needs administrator rights",
            ));
        }

        let (name, url, digest, args): (String, String, Option<String>, Vec<&str>) =
            if let Some(arch) = id.strip_prefix("vc-redist-") {
                if !["x64", "x86", "arm64"].contains(&arch) {
                    return Err(Error::invalid_package_id(id, "unknown architecture"));
                }
                (
                    format!("Visual C++ Redistributable ({arch})"),
                    vc_url(arch),
                    None,
                    vec!["/install", "/quiet", "/norestart"],
                )
            } else if id == "directx-legacy" {
                (
                    "DirectX End-User Runtime".into(),
                    DIRECTX_WEB_SETUP.into(),
                    None,
                    vec!["/Q"],
                )
            } else if let Some(rest) = id.strip_prefix("dotnet-") {
                let (product, line) = rest
                    .split_once('-')
                    .ok_or_else(|| Error::invalid_package_id(id, "malformed .NET id"))?;
                if !["windowsdesktop", "runtime"].contains(&product) {
                    return Err(Error::invalid_package_id(id, "unknown .NET product"));
                }
                let client = http::client(None, Duration::from_secs(30))?;
                let channels = parse_dotnet_index(&http::get_json(&client, DOTNET_INDEX).await?);
                let channel = channels
                    .into_iter()
                    .find(|c| c.channel == line)
                    .ok_or_else(|| Error::invalid_package_id(id, "unknown .NET line"))?;
                if channel.support_phase == "eol" {
                    return Err(Error::unavailable(
                        "components",
                        format!(".NET {line} is out of support; Microsoft ships no more patches"),
                    ));
                }
                let releases = http::get_json(&client, &channel.releases_json).await?;
                let (version, url, sha512) = dotnet_installer(&releases, product, rid())
                    .ok_or_else(|| {
                        Error::unavailable("components", "no installer for this machine")
                    })?;
                (
                    format!(".NET {product} {version}"),
                    url,
                    Some(sha512),
                    vec!["/install", "/quiet", "/norestart"],
                )
            } else {
                return Err(Error::invalid_package_id(id, "not a known component"));
            };

        let dir = http::download_dir()?.join(format!(
            "component-{id}-{}",
            chrono::Utc::now().timestamp_millis()
        ));
        tokio::fs::create_dir_all(&dir).await?;
        let file = dir.join(http::file_name_for(&url, "installer.exe"));

        let result = async {
            let client = http::client(None, Duration::from_secs(60 * 60))?;
            http::download(&client, &url, &file, |_, _| {}).await?;
            let lock = lock_and_verify(
                &file,
                digest.as_deref().map(ExpectedDigest::Sha512),
                MICROSOFT,
                &name,
            )
            .await?;
            let path = file.to_string_lossy().to_string();
            let out = odysync_core::proc::run(&path, &args, INSTALL_TIMEOUT).await;
            drop(lock);
            let out = out?;
            // 1638: a newer version is already installed — for "install the
            // latest" that is the answer, not a failure. 3010: restart needed.
            match out.code {
                0 => Ok(format!("{name} installed")),
                3010 | 1641 => Ok(format!("{name} installed; restart to finish")),
                1638 => Ok(format!("{name} is already up to date")),
                code => Err(Error::CommandFailed {
                    command: name.clone(),
                    code,
                    stderr: out.stderr.chars().take(300).collect(),
                }),
            }
        }
        .await;

        let _ = tokio::fs::remove_dir_all(&dir).await;
        result
    }
}

/// Every component Odysync tracks on this machine.
pub async fn status() -> Vec<ComponentStatus> {
    #[cfg(windows)]
    {
        windows_impl::status().await
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Install or repair the component with this id.
pub async fn install(id: &str) -> odysync_core::error::Result<String> {
    #[cfg(windows)]
    {
        windows_impl::install(id).await
    }
    #[cfg(not(windows))]
    {
        Err(odysync_core::error::Error::unavailable(
            "components",
            format!("{id} is a Windows component"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vc_versions_lose_their_padding() {
        assert_eq!(normalize_vc_version("v14.51.36247.00"), "14.51.36247.0");
        assert_eq!(normalize_vc_version("14.40.33810.00"), "14.40.33810.0");
    }

    #[test]
    fn dotnet_versions_belong_to_their_major_line() {
        assert_eq!(dotnet_channel_of("8.0.29").as_deref(), Some("8.0"));
        assert_eq!(dotnet_channel_of("10.0.10").as_deref(), Some("10.0"));
        assert_eq!(dotnet_channel_of("garbage"), None);
    }

    #[test]
    fn the_newest_patch_per_line_is_kept_and_previews_ignored() {
        let found = newest_per_channel(&[
            "8.0.11".into(),
            "8.0.29".into(),
            "8.0.3".into(),
            "10.0.10".into(),
            "11.0.0-rc.2.26504.105".into(),
        ]);
        assert_eq!(found, vec!["10.0.10".to_string(), "8.0.29".to_string()]);
    }

    #[test]
    fn the_release_index_is_read() {
        let index = serde_json::json!({ "releases-index": [
            { "channel-version": "10.0", "latest-runtime": "10.0.13", "support-phase": "active",
              "releases.json": "https://builds.dotnet.microsoft.com/dotnet/release-metadata/10.0/releases.json" },
            { "channel-version": "6.0", "latest-runtime": "6.0.36", "support-phase": "eol",
              "releases.json": "https://builds.dotnet.microsoft.com/dotnet/release-metadata/6.0/releases.json" }
        ]});
        let channels = parse_dotnet_index(&index);
        assert_eq!(channels.len(), 2);
        assert_eq!(channels[0].latest_runtime, "10.0.13");
        assert_eq!(channels[1].support_phase, "eol");
    }

    fn releases(url: &str, hash: &str) -> serde_json::Value {
        serde_json::json!({ "releases": [ { "windowsdesktop": {
            "version": "8.0.32",
            "files": [
                { "name": "windowsdesktop-runtime-win-x64.zip", "rid": "win-x64",
                  "url": "https://builds.dotnet.microsoft.com/x.zip", "hash": "a".repeat(128) },
                { "name": "windowsdesktop-runtime-win-x64.exe", "rid": "win-x64",
                  "url": url, "hash": hash }
            ]
        } } ] })
    }

    #[test]
    fn the_exe_installer_and_its_sha512_are_picked() {
        let hash = "B".repeat(128);
        let json = releases(
            "https://builds.dotnet.microsoft.com/dotnet/WindowsDesktop/8.0.32/x.exe",
            &hash,
        );
        let (version, url, sha) =
            dotnet_installer(&json, "windowsdesktop", "win-x64").expect("installer");
        assert_eq!(version, "8.0.32");
        assert!(url.ends_with(".exe"));
        assert_eq!(sha, "b".repeat(128));
        assert!(dotnet_installer(&json, "windowsdesktop", "win-arm64").is_none());
    }

    #[test]
    fn an_installer_off_microsofts_hosts_or_without_a_full_hash_is_refused() {
        assert!(
            dotnet_installer(
                &releases("https://example.com/x.exe", &"b".repeat(128)),
                "windowsdesktop",
                "win-x64"
            )
            .is_none()
        );
        assert!(
            dotnet_installer(
                &releases("https://builds.dotnet.microsoft.com/x.exe", "abc"),
                "windowsdesktop",
                "win-x64"
            )
            .is_none()
        );
    }
}
