//! AppImage update backend for Linux.
//!
//! AppImages are self-contained Linux applications that don't require
//! installation. Some AppImages embed update information (via AppImageUpdate
//! or the zsync protocol). This backend scans common directories for
//! AppImage files and checks for available updates.
//!
//! Commands used:
//!   - Walk ~/Applications, ~/Downloads, /opt/appimages for *.AppImage files
//!   - `<appimage> --appimage-extract-and-run --appimage-update-information` — get update info
//!   - `AppImageUpdate <appimage>` — update in place
//!
//! Reference: https://appimage.org/

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;


const INSTALL_TIMEOUT: Duration = Duration::from_secs(300);


/// Directories to scan for AppImage files.
const SCAN_DIRS: &[&str] = &[
    "~/Applications",
    "~/Downloads",
    "/opt/appimages",
    "/usr/local/bin",
];

pub struct AppImageBackend;

impl AppImageBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AppImageBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for AppImageBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::AppImage
    }

    fn display_name(&self) -> &str {
        "AppImage"
    }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && !find_appimages().is_empty()
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let appimages = find_appimages();
        let mut candidates = Vec::with_capacity(appimages.len());

        for path in &appimages {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string());

            // Try to extract version from filename.
            // Common patterns: "App-1.2.3.AppImage", "App_1.2.3_x86_64.AppImage"
            let installed_version = extract_version_from_filename(&name);

            let id_str = path.to_string_lossy().to_string();
            candidates.push(UpdateCandidate {
                id: PackageId::new(BackendKind::AppImage, &id_str),
                name,
                installed: Version::parse(installed_version.as_deref().unwrap_or("")),
                available: Version::parse(""), // AppImage doesn't expose available version without running it
                size_bytes: path.metadata().ok().map(|m| m.len()),
                expected_sha256: None,
            });
        }

        Ok(candidates)
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }

        let path = &candidate.id.native;

        // Try AppImageUpdate first (dedicated tool).
        if proc::exists("AppImageUpdate", &["--version"]).await {
            let out = proc::run("AppImageUpdate", &[path], INSTALL_TIMEOUT).await?;
            if out.success() {
                return Ok(());
            }
            tracing::warn!(stderr = %out.stderr, "AppImageUpdate failed, trying self-update");
        }

        // Try the AppImage's built-in update mechanism.
        let out = proc::run(path, &["--appimage-update"], INSTALL_TIMEOUT).await?;

        if out.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                command: format!("{} --appimage-update", path),
                code: out.code,
                stderr: if out.stderr.trim().is_empty() {
                    out.stdout
                } else {
                    out.stderr
                },
            })
        }
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let path = PathBuf::from(&candidate.id.native);
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        Ok(extract_version_from_filename(&name))
    }
}

/// Find all .AppImage files in known directories.
fn find_appimages() -> Vec<PathBuf> {
    let mut results = Vec::new();

    for dir in SCAN_DIRS {
        let expanded = expand_tilde(dir);
        if let Ok(entries) = std::fs::read_dir(&expanded) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "AppImage").unwrap_or(false) {
                    results.push(path);
                }
            }
        }
    }

    results
}

/// Expand `~` to the user's home directory.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

/// Extract a version string from an AppImage filename.
///
/// Common patterns:
/// - `App-1.2.3` → `1.2.3`
/// - `App_1.2.3_x86_64` → `1.2.3`
/// - `App-1.2.3-x86_64` → `1.2.3`
fn extract_version_from_filename(name: &str) -> Option<String> {
    // Try splitting by '-' or '_' and find a segment that looks like a version.
    for sep in &['-', '_'] {
        let parts: Vec<&str> = name.split(*sep).collect();
        for part in parts.iter().skip(1) {
            // A version segment starts with a digit and contains at least one dot.
            if part.starts_with(|c: char| c.is_ascii_digit()) && part.contains('.') {
                // Strip trailing architecture suffixes.
                let version = part
                    .trim_end_matches("_x86_64")
                    .trim_end_matches("-x86_64")
                    .trim_end_matches("_aarch64")
                    .trim_end_matches("-aarch64")
                    .trim_end_matches("_arm64")
                    .trim_end_matches("-arm64");
                return Some(version.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_version_from_dash_separated() {
        assert_eq!(
            extract_version_from_filename("Firefox-141.0"),
            Some("141.0".into())
        );
        assert_eq!(
            extract_version_from_filename("App-2.5.1-x86_64"),
            Some("2.5.1".into())
        );
    }

    #[test]
    fn extracts_version_from_underscore_separated() {
        assert_eq!(
            extract_version_from_filename("Firefox_141.0_x86_64"),
            Some("141.0".into())
        );
        assert_eq!(
            extract_version_from_filename("App_3.0.1_aarch64"),
            Some("3.0.1".into())
        );
    }

    #[test]
    fn returns_none_when_no_version_found() {
        assert_eq!(extract_version_from_filename("MyApp"), None);
        assert_eq!(extract_version_from_filename("MyApp-latest"), None);
    }

    #[test]
    fn backend_kind_is_correct() {
        let b = AppImageBackend::new();
        assert_eq!(b.kind(), BackendKind::AppImage);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = AppImageBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = AppImageBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::AppImage, "/home/user/App.AppImage"),
            name: "App".into(),
            installed: Version::parse("1.0"),
            available: Version::parse(""),
            size_bytes: None,
            expected_sha256: None,
        };
        let err = backend.apply(&candidate).await.unwrap_err();
        assert!(matches!(err, Error::Verification { .. }));
    }
}
