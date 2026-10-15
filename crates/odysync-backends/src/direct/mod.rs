//! Updates fetched straight from the publisher — no package manager required.
//!
//! This is what lets Odysync update software on a machine where winget is
//! missing, broken or simply not wanted. It reads the OS's own list of
//! installed programs, recognises the ones in a curated [`catalog`], asks each
//! publisher's own release feed for the latest version, and — only when asked
//! to apply — downloads the installer and runs it silently.
//!
//! ## The checks between a download and running it
//!
//! In order, every one of them fatal:
//!
//! 1. **Same version as the scan.** The release is re-read at apply time and
//!    must be exactly the version the user approved. If the publisher shipped
//!    another release in between, the update stops and asks for a rescan
//!    rather than installing something nobody looked at.
//! 2. **HTTPS, from the publisher.** A GitHub asset URL must belong to the
//!    repository the catalog names; a vendor URL must start with the prefix
//!    the catalog pins. Redirects to plain HTTP are refused.
//! 3. **The file is locked**, then re-hashed from disk: nothing else can write
//!    to or replace it until the installer has finished. This closes the gap
//!    in which an unprivileged process could swap a verified installer for
//!    another one just before Odysync runs it elevated.
//! 4. **SHA-256**, whenever the source publishes one (GitHub does for every
//!    release asset, VS Code and Node.js do too).
//! 5. **The publisher's signature.** Valid, *and* issued to the signer the
//!    catalog pins for that app — see [`signer`].
//!
//! And around it: the program must not be running, and an installer only runs
//! over an install of its own kind (see [`catalog::InstallerKind::compatible_with`]).

pub mod catalog;
pub mod http;
pub mod inventory;
pub mod signer;

use std::time::Duration;

use async_trait::async_trait;

use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{
    BackendKind, InstalledPackage, PackageId, UpdateCandidate, DIRECT_MACHINE_SCOPE,
    DIRECT_USER_SCOPE,
};
use odysync_core::version::Version;

use catalog::{Catalog, CatalogApp, InstalledApp, Release, Scope, Source};

/// How long one installer may run before it is presumed hung.
#[cfg(windows)]
const INSTALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub struct DirectBackend {
    proxy_url: Option<String>,
}

impl DirectBackend {
    pub fn new() -> Self {
        Self { proxy_url: None }
    }

    pub fn with_proxy(proxy_url: Option<String>) -> Self {
        Self { proxy_url }
    }

    fn client(&self, timeout: Duration) -> Result<reqwest::Client> {
        http::client(self.proxy_url.as_deref(), timeout)
    }
}

impl Default for DirectBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// The native id for an app: its scope, then its catalog id.
pub fn native_id(scope: Scope, entry: &CatalogApp) -> String {
    let prefix = match scope {
        Scope::Machine => DIRECT_MACHINE_SCOPE,
        Scope::User => DIRECT_USER_SCOPE,
    };
    format!("{prefix}{}", entry.id)
}

/// Split a native id back into its scope and catalog id.
pub fn parse_native_id(native: &str) -> Option<(Scope, &str)> {
    if let Some(id) = native.strip_prefix(DIRECT_MACHINE_SCOPE) {
        Some((Scope::Machine, id))
    } else {
        native
            .strip_prefix(DIRECT_USER_SCOPE)
            .map(|id| (Scope::User, id))
    }
}

/// The installed programs the catalog knows, each with its entry.
fn recognised(catalog: &Catalog, installed: Vec<InstalledApp>) -> Vec<(InstalledApp, CatalogApp)> {
    let mut out: Vec<(InstalledApp, CatalogApp)> = Vec::new();
    for app in installed {
        if let Some(entry) = catalog.match_installed(&app) {
            // One candidate per (app, scope). A second uninstall entry for the
            // same app and scope is a leftover, not a second install.
            if !out
                .iter()
                .any(|(a, e)| e.id == entry.id && a.scope == app.scope)
            {
                out.push((app, entry.clone()));
            }
        }
    }
    out
}

/// Ask the entry's source for its latest release, for this install.
async fn resolve(
    client: &reqwest::Client,
    entry: &CatalogApp,
    app: &InstalledApp,
) -> Result<Option<Release>> {
    match &entry.source {
        Source::Github {
            repo,
            tag,
            assets,
            user_assets,
        } => {
            let Some(pattern) = catalog::asset_pattern(assets, user_assets, app.scope, app.arch)
            else {
                return Ok(None);
            };
            let url = format!("https://api.github.com/repos/{repo}/releases/latest");
            let release = http::get_json(client, &url).await?;
            if release.get("draft").and_then(|v| v.as_bool()) == Some(true)
                || release.get("prerelease").and_then(|v| v.as_bool()) == Some(true)
            {
                return Ok(None);
            }
            let tag_name = release
                .get("tag_name")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let Some(version) = catalog::version_from_tag(tag, tag_name) else {
                tracing::debug!(app = %entry.id, tag = tag_name, "tag does not match the catalog pattern");
                return Ok(None);
            };
            let asset_re = regex::Regex::new(pattern)
                .map_err(|e| Error::parse("catalog asset pattern", e.to_string()))?;
            let Some(asset) = release
                .get("assets")
                .and_then(|a| a.as_array())
                .into_iter()
                .flatten()
                .find(|a| {
                    a.get("name")
                        .and_then(|n| n.as_str())
                        .is_some_and(|n| asset_re.is_match(n))
                })
            else {
                return Ok(None);
            };
            let url = asset
                .get("browser_download_url")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            if !catalog::is_release_asset_of(repo, &url) {
                return Err(Error::security(
                    "direct",
                    format!("{}: asset URL is not a release of {repo}: {url}", entry.id),
                ));
            }
            Ok(Some(Release {
                version,
                url,
                sha256: catalog::sha256_from_digest(asset.get("digest").and_then(|v| v.as_str())),
                size_bytes: asset.get("size").and_then(|v| v.as_u64()),
            }))
        }

        Source::Json {
            endpoints,
            version_field,
            url_field,
            sha256_field,
            download_prefix,
        } => {
            let key = format!("{}-{}", app.scope.as_str(), app.arch);
            let Some(endpoint) = endpoints.get(&key) else {
                return Ok(None);
            };
            let body = http::get_json(client, endpoint).await?;
            let field = |name: &str| body.get(name).and_then(|v| v.as_str()).map(str::trim);
            let (Some(version), Some(url)) = (field(version_field), field(url_field)) else {
                return Err(Error::parse(
                    format!("{} update feed", entry.id),
                    "missing version or url",
                ));
            };
            if !url.starts_with(download_prefix.as_str()) {
                return Err(Error::security(
                    "direct",
                    format!("{}: download URL is outside {download_prefix}", entry.id),
                ));
            }
            let sha256 = sha256_field
                .as_deref()
                .and_then(field)
                .filter(|h| catalog::is_sha256_hex(h))
                .map(str::to_ascii_lowercase);
            Ok(Some(Release {
                version: version.to_string(),
                url: url.to_string(),
                sha256,
                size_bytes: None,
            }))
        }

        Source::Nodejs => {
            let Some(installed) = app.display_version.as_deref() else {
                return Ok(None);
            };
            let index = http::get_json(client, "https://nodejs.org/dist/index.json").await?;
            let Some(version) = catalog::pick_nodejs_release(&index, installed) else {
                return Ok(None);
            };
            let file = format!("node-v{version}-{}.msi", app.arch);
            let base = format!("https://nodejs.org/dist/v{version}/");
            let shasums = http::get_text(client, &format!("{base}SHASUMS256.txt")).await?;
            let Some(sha256) = catalog::sha256_from_shasums(&shasums, &file) else {
                // No published hash means no installer for this arch in this
                // release; not offering one is the honest answer.
                return Ok(None);
            };
            Ok(Some(Release {
                version,
                url: format!("{base}{file}"),
                sha256: Some(sha256),
                size_bytes: None,
            }))
        }
    }
}

fn candidate_for(app: &InstalledApp, entry: &CatalogApp, release: &Release) -> UpdateCandidate {
    UpdateCandidate {
        id: PackageId::new(BackendKind::Direct, native_id(app.scope, entry)).with_alias(&entry.id),
        name: entry.name.clone(),
        installed: Version::parse(app.display_version.as_deref().unwrap_or("")),
        available: Version::parse(&release.version),
        size_bytes: release.size_bytes,
        expected_sha256: release.sha256.clone(),
    }
}

#[async_trait]
impl Backend for DirectBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Direct
    }

    fn display_name(&self) -> &str {
        "Direct from publisher"
    }

    async fn is_available(&self) -> bool {
        // Needs nothing installed — only the OS's own program list, which
        // exists on every supported Windows.
        cfg!(windows)
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let catalog = Catalog::embedded();
        let found = recognised(&catalog, installed_apps().await);
        if found.is_empty() {
            return Ok(Vec::new());
        }

        let client = self.client(Duration::from_secs(60))?;
        let lookups = found.iter().map(|(app, entry)| {
            let client = &client;
            async move { (app, entry, resolve(client, entry, app).await) }
        });

        let mut out = Vec::new();
        for (app, entry, result) in futures::future::join_all(lookups).await {
            match result {
                Ok(Some(release)) => out.push(candidate_for(app, entry, &release)),
                Ok(None) => {}
                // One publisher being unreachable must not hide the others.
                Err(e) => {
                    tracing::warn!(app = %entry.id, error = %e, "could not read the latest release")
                }
            }
        }
        Ok(out)
    }

    async fn list_installed(&self) -> Result<Vec<InstalledPackage>> {
        let catalog = Catalog::embedded();
        Ok(recognised(&catalog, installed_apps().await)
            .into_iter()
            .map(|(app, entry)| InstalledPackage {
                id: PackageId::new(BackendKind::Direct, native_id(app.scope, &entry))
                    .with_alias(&entry.id),
                name: entry.name.clone(),
                version: app.display_version.unwrap_or_default(),
            })
            .collect())
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        #[cfg(windows)]
        {
            windows_apply::apply(self, candidate).await
        }
        #[cfg(not(windows))]
        {
            Err(Error::unavailable(
                "direct",
                format!("{} cannot be installed on this platform", candidate.name),
            ))
        }
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let Some((scope, id)) = parse_native_id(&candidate.id.native) else {
            return Ok(None);
        };
        let catalog = Catalog::embedded();
        let Some(entry) = catalog.find(id).cloned() else {
            return Ok(None);
        };
        Ok(installed_apps()
            .await
            .into_iter()
            .filter(|a| a.scope == scope && entry.matches(a))
            .filter_map(|a| a.display_version)
            // Several matching entries means a leftover next to the real
            // install; the newest one is the one that was just written.
            .max_by(|a, b| {
                Version::parse(a)
                    .compare(&Version::parse(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            }))
    }
}

/// The OS's list of installed programs, read off the async runtime.
async fn installed_apps() -> Vec<InstalledApp> {
    tokio::task::spawn_blocking(inventory::installed_apps)
        .await
        .unwrap_or_default()
}

#[cfg(windows)]
pub mod windows_apply {
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::Path;

    use sha2::{Digest, Sha256};

    use super::*;

    /// A digest the publisher published for a download.
    #[derive(Debug, Clone, Copy)]
    pub enum ExpectedDigest<'a> {
        Sha256(&'a str),
        /// Microsoft publishes SHA-512 for .NET downloads.
        Sha512(&'a str),
    }

    /// `FILE_SHARE_READ`: others may read the file while it is held open, but
    /// not write to it, rename it or delete it.
    const FILE_SHARE_READ: u32 = 0x0000_0001;

    /// Lock `file`, then check it is what the publisher shipped.
    ///
    /// The returned handle denies other processes write, rename and delete
    /// access; keep it alive until the installer has exited, so the file that
    /// runs is the file that was checked. Hashing happens *under* the lock,
    /// from disk, rather than trusting the digest computed while downloading.
    ///
    /// Shared by everything that runs an installer Odysync downloaded itself.
    pub async fn lock_and_verify(
        file: &Path,
        expected_digest: Option<ExpectedDigest<'_>>,
        expected_signer: &str,
        name: &str,
    ) -> Result<std::fs::File> {
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(file)?;
        let reader = lock.try_clone()?;
        let (sha256, sha512) =
            tokio::task::spawn_blocking(move || -> std::io::Result<(String, String)> {
                use std::io::Read;
                let mut reader = std::io::BufReader::with_capacity(1 << 20, reader);
                let mut h256 = Sha256::new();
                let mut h512 = sha2::Sha512::new();
                let mut buf = vec![0u8; 1 << 20];
                loop {
                    let n = reader.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    h256.update(&buf[..n]);
                    h512.update(&buf[..n]);
                }
                Ok((hex::encode(h256.finalize()), hex::encode(h512.finalize())))
            })
            .await
            .map_err(|e| Error::Io(std::io::Error::other(e)))??;

        let refuse = |detail: String| Error::Verification {
            package: name.to_string(),
            detail,
        };

        let (algorithm, expected, actual) = match expected_digest {
            None => ("", "", ""),
            Some(ExpectedDigest::Sha256(e)) => ("SHA-256", e, sha256.as_str()),
            Some(ExpectedDigest::Sha512(e)) => ("SHA-512", e, sha512.as_str()),
        };
        if !algorithm.is_empty() && !actual.eq_ignore_ascii_case(expected.trim()) {
            return Err(refuse(format!(
                "{algorithm} mismatch: expected {expected}, got {actual}"
            )));
        }

        match odysync_verify::verify_signature(file).await {
            odysync_verify::SignatureStatus::Valid { subject } => {
                if !signer::signer_matches(&subject, expected_signer) {
                    return Err(refuse(format!(
                        "signed by {subject:?}, but {expected_signer:?} was expected"
                    )));
                }
                tracing::info!(name, %subject, "installer signature verified");
                Ok(lock)
            }
            odysync_verify::SignatureStatus::Unsigned => {
                Err(refuse("the installer is not signed".into()))
            }
            odysync_verify::SignatureStatus::Invalid { detail } => Err(refuse(format!(
                "the installer's signature does not validate: {detail}"
            ))),
            odysync_verify::SignatureStatus::Unsupported => Err(refuse(
                "signatures cannot be checked on this platform".into(),
            )),
        }
    }

    pub(super) async fn apply(backend: &DirectBackend, candidate: &UpdateCandidate) -> Result<()> {
        candidate.id.validate()?;
        let (scope, id) = parse_native_id(&candidate.id.native).ok_or_else(|| {
            Error::invalid_package_id(&candidate.id.native, "not a direct package id")
        })?;
        let catalog = Catalog::embedded();
        let entry = catalog
            .find(id)
            .cloned()
            .ok_or_else(|| Error::invalid_package_id(id, "not in the catalog"))?;

        // The install as it is right now, not as the scan saw it.
        let installed = tokio::task::spawn_blocking(inventory::installed_apps)
            .await
            .unwrap_or_default();
        let app = installed
            .into_iter()
            .find(|a| a.scope == scope && entry.matches(a))
            .ok_or_else(|| {
                Error::unavailable(
                    "direct",
                    format!("{} is no longer installed for this scope", entry.name),
                )
            })?;

        // 1. Exactly the version that was approved.
        let client = backend.client(Duration::from_secs(60))?;
        let release = resolve(&client, &entry, &app).await?.ok_or_else(|| {
            Error::unavailable(
                "direct",
                format!(
                    "{} no longer offers an installer for this machine",
                    entry.name
                ),
            )
        })?;
        if release.version.trim() != candidate.available.raw().trim() {
            return Err(Error::Verification {
                package: entry.name.clone(),
                detail: format!(
                    "the publisher now offers {} instead of the approved {}; scan again",
                    release.version,
                    candidate.available.raw()
                ),
            });
        }

        // Never replace files under a running program.
        if let Some(dir) = &app.install_dir {
            let running = inventory::running_executables().await;
            if let Some(exe) = inventory::running_inside(dir, &running) {
                return Err(Error::unavailable(
                    "direct",
                    format!(
                        "{} is running ({}); close it and try again",
                        entry.name,
                        exe.display()
                    ),
                ));
            }
        }

        // A fresh directory per update, so nothing left from an earlier
        // attempt can be picked up by mistake.
        let dir = http::download_dir()?.join(format!(
            "{}-{}-{}",
            entry.id,
            release.version,
            chrono::Utc::now().timestamp_millis()
        ));
        tokio::fs::create_dir_all(&dir).await?;
        let file = dir.join(http::file_name_for(&release.url, "installer.bin"));

        let result = download_verify_install(
            backend,
            &client_for_download(backend)?,
            &entry,
            &release,
            &file,
        )
        .await;

        let _ = tokio::fs::remove_dir_all(&dir).await;
        result
    }

    fn client_for_download(backend: &DirectBackend) -> Result<reqwest::Client> {
        backend.client(Duration::from_secs(60 * 60))
    }

    async fn download_verify_install(
        _backend: &DirectBackend,
        client: &reqwest::Client,
        entry: &CatalogApp,
        release: &Release,
        file: &Path,
    ) -> Result<()> {
        // 2. HTTPS from the publisher: enforced by the resolver (URL shape)
        //    and the client (no redirect to plain HTTP).
        tracing::info!(app = %entry.id, url = %release.url, "downloading installer");
        http::download(client, &release.url, file, |_, _| {}).await?;

        // 3–5. Lock, re-hash, check the hash and the publisher's signature.
        let lock = lock_and_verify(
            file,
            release.sha256.as_deref().map(ExpectedDigest::Sha256),
            &entry.signer,
            &entry.name,
        )
        .await?;

        let file_arg = file.to_string_lossy().to_string();
        let (program, args): (String, Vec<String>) = match entry.installer {
            catalog::InstallerKind::Msi => {
                let mut args = vec!["/i".to_string(), file_arg];
                args.extend(entry.installer.silent_args().iter().map(|s| s.to_string()));
                args.extend(entry.extra_args.iter().cloned());
                ("msiexec.exe".to_string(), args)
            }
            kind => {
                let mut args: Vec<String> =
                    kind.silent_args().iter().map(|s| s.to_string()).collect();
                args.extend(entry.extra_args.iter().cloned());
                (file_arg, args)
            }
        };

        tracing::info!(app = %entry.id, version = %release.version, "running installer");
        let out = odysync_core::proc::run(&program, &args, INSTALL_TIMEOUT).await;
        drop(lock);
        let out = out?;

        // 0 is success; 3010 and 1641 are Windows Installer's "succeeded,
        // restart required/initiated". The runner asks the OS whether a
        // restart is pending afterwards, so it is reported either way.
        match out.code {
            0 | 3010 | 1641 => Ok(()),
            code => Err(Error::CommandFailed {
                command: format!("{} installer", entry.name),
                code,
                stderr: out.stderr.chars().take(500).collect(),
            }),
        }
    }
}

#[cfg(all(test, windows))]
mod verify_tests {
    use super::windows_apply::{lock_and_verify, ExpectedDigest};
    use std::path::PathBuf;

    /// A Microsoft-signed file every Windows machine has, copied somewhere
    /// writable so it can be tampered with.
    fn signed_copy(tag: &str) -> (tempfile::TempDir, PathBuf) {
        let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let source = PathBuf::from(windir).join(r"System32\msiexec.exe");
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join(format!("{tag}.exe"));
        std::fs::copy(&source, &dest).unwrap();
        (dir, dest)
    }

    fn sha256_of(path: &std::path::Path) -> String {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(std::fs::read(path).unwrap()))
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn the_right_publisher_and_hash_pass() {
        let (_dir, file) = signed_copy("good");
        let sha = sha256_of(&file);
        let lock = rt()
            .block_on(lock_and_verify(
                &file,
                Some(ExpectedDigest::Sha256(&sha)),
                "Microsoft Windows",
                "test",
            ))
            .expect("a genuine Microsoft file verifies");
        // While the lock is held the file cannot be replaced or deleted.
        assert!(std::fs::write(&file, b"swapped").is_err());
        assert!(std::fs::remove_file(&file).is_err());
        drop(lock);
        assert!(std::fs::remove_file(&file).is_ok());
    }

    #[test]
    fn a_valid_signature_from_someone_else_is_refused() {
        let (_dir, file) = signed_copy("other");
        let err = rt()
            .block_on(lock_and_verify(&file, None, "Mozilla Corporation", "test"))
            .expect_err("Microsoft is not Mozilla");
        assert!(err.to_string().contains("was expected"), "{err}");
    }

    #[test]
    fn a_hash_mismatch_is_refused_before_the_signature_is_consulted() {
        let (_dir, file) = signed_copy("hash");
        let err = rt()
            .block_on(lock_and_verify(
                &file,
                Some(ExpectedDigest::Sha256(&"0".repeat(64))),
                "Microsoft Windows",
                "test",
            ))
            .expect_err("wrong hash");
        assert!(err.to_string().contains("SHA-256 mismatch"), "{err}");

        let err = rt()
            .block_on(lock_and_verify(
                &file,
                Some(ExpectedDigest::Sha512(&"0".repeat(128))),
                "Microsoft Windows",
                "test",
            ))
            .expect_err("wrong SHA-512");
        assert!(err.to_string().contains("SHA-512 mismatch"), "{err}");
    }

    #[test]
    fn a_tampered_file_is_refused() {
        let (_dir, file) = signed_copy("tampered");
        let mut bytes = std::fs::read(&file).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xFF;
        std::fs::write(&file, &bytes).unwrap();
        let err = rt()
            .block_on(lock_and_verify(&file, None, "Microsoft Windows", "test"))
            .expect_err("a changed byte must not verify");
        let text = err.to_string();
        assert!(
            text.contains("not signed") || text.contains("does not validate"),
            "{text}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catalog::Technology;

    #[test]
    fn native_ids_round_trip_with_their_scope() {
        let catalog = Catalog::embedded();
        let entry = catalog.find("git").expect("git is in the catalog");
        assert_eq!(native_id(Scope::Machine, entry), "machine:git");
        assert_eq!(native_id(Scope::User, entry), "user:git");
        assert_eq!(
            parse_native_id("machine:git"),
            Some((Scope::Machine, "git"))
        );
        assert_eq!(
            parse_native_id("user:vscode"),
            Some((Scope::User, "vscode"))
        );
        assert_eq!(parse_native_id("git"), None);
    }

    /// The policy engine decides elevation from the candidate, so a per-user
    /// install must never be blocked for want of administrator rights.
    #[test]
    fn only_machine_wide_installs_need_elevation() {
        let catalog = Catalog::embedded();
        let entry = catalog.find("vscode").unwrap();
        let app = |scope| InstalledApp {
            display_name: "Microsoft Visual Studio Code (User)".into(),
            display_version: Some("1.140.0".into()),
            publisher: Some("Microsoft Corporation".into()),
            scope,
            arch: "x64",
            technology: Technology::Inno,
            install_dir: None,
        };
        let release = Release {
            version: "1.142.0".into(),
            url: "https://vscode.download.prss.microsoft.com/x.exe".into(),
            sha256: None,
            size_bytes: None,
        };
        assert!(!candidate_for(&app(Scope::User), entry, &release).requires_elevation());
        assert!(candidate_for(&app(Scope::Machine), entry, &release).requires_elevation());

        let c = candidate_for(&app(Scope::User), entry, &release);
        assert!(c.id.is_valid());
        // Holds can name the app by its catalog id.
        assert_eq!(c.id.alias.as_deref(), Some("vscode"));
    }

    #[test]
    fn a_leftover_second_entry_does_not_produce_a_second_candidate() {
        let catalog = Catalog::embedded();
        let git = |dir: &str| InstalledApp {
            display_name: "Git".into(),
            display_version: Some("2.56.0".into()),
            publisher: Some("The Git Development Community".into()),
            scope: Scope::Machine,
            arch: "x64",
            technology: Technology::Inno,
            install_dir: Some(dir.into()),
        };
        let found = recognised(&catalog, vec![git("C:/a"), git("C:/b")]);
        assert_eq!(found.len(), 1);
    }
}
