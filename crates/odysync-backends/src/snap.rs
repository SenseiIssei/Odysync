//! Snap backend for Linux.
//!
//! Snap packages are containerised applications with automatic update support.
//! We use `snap refresh --list` to find pending updates and `snap refresh` to
//! apply them. Snap verifies package signatures via its store infrastructure.
//!
//! Commands used:
//!   - `snap refresh --list` — list pending updates
//!   - `snap refresh <name>` — update a specific snap
//!   - `snap list <name>` — read installed version

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

const SCAN_TIMEOUT: Duration = Duration::from_secs(60);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);
const QUERY_TIMEOUT: Duration = Duration::from_secs(30);

pub struct SnapBackend;

impl SnapBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SnapBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for SnapBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Snap
    }

    fn display_name(&self) -> &str {
        "Snap"
    }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && proc::exists("snap", &["version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let out = proc::run("snap", &["refresh", "--list"], SCAN_TIMEOUT).await?;

        if !out.success() {
            return Err(Error::CommandFailed {
                command: "snap refresh --list".into(),
                code: out.code,
                stderr: out.stderr,
            });
        }

        Ok(parse_snap_refresh_list(&out.stdout))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }

        let out = proc::run("snap", &["refresh", &candidate.id.native], INSTALL_TIMEOUT).await?;

        if out.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                command: format!("snap refresh {}", candidate.id.native),
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
        let out = proc::run("snap", &["list", &candidate.id.native], QUERY_TIMEOUT).await?;

        if !out.success() {
            return Ok(None);
        }

        // Output: "Name  Version  Rev  Tracking  Publisher  Notes"
        // The version is the second column (after header).
        let line = out.stdout.lines().find(|l| {
            let l = l.trim();
            !l.is_empty() && !l.starts_with("Name")
        });
        let version = line.and_then(|l| l.split_whitespace().nth(1)).unwrap_or("");

        if version.is_empty() {
            Ok(None)
        } else {
            Ok(Some(version.to_string()))
        }
    }
}

/// Parse `snap refresh --list` output.
///
/// Format (with header):
/// ```text
/// Name      Version  Rev  Tracking  Publisher  Notes
/// firefox   141.0    4173 latest/stable  mozilla
/// ```
fn parse_snap_refresh_list(stdout: &str) -> Vec<UpdateCandidate> {
    stdout
        .lines()
        .skip_while(|l| l.trim().starts_with("Name") || l.trim().is_empty())
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }

            let mut parts = line.split_whitespace();
            let name = parts.next()?;
            let version = parts.next()?;

            Some(UpdateCandidate {
                id: PackageId::new(BackendKind::Snap, name),
                name: name.to_string(),
                installed: Version::parse(""),
                available: Version::parse(version),
                size_bytes: None,
                expected_sha256: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_snap_refresh_list() {
        let output = "\
Name      Version  Rev  Tracking       Publisher  Notes
firefox   141.0    4173 latest/stable  mozilla
chromium  120.0    2663 latest/stable  canonical
";
        let candidates = parse_snap_refresh_list(output);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].id.native, "firefox");
        assert_eq!(candidates[0].available.raw(), "141.0");
        assert_eq!(candidates[1].id.native, "chromium");
    }

    #[test]
    fn empty_output_yields_empty_vec() {
        assert!(parse_snap_refresh_list("").is_empty());
    }

    #[test]
    fn header_only_yields_empty_vec() {
        assert!(
            parse_snap_refresh_list("Name      Version  Rev  Tracking  Publisher  Notes\n")
                .is_empty()
        );
    }

    #[test]
    fn backend_kind_is_correct() {
        let b = SnapBackend::new();
        assert_eq!(b.kind(), BackendKind::Snap);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = SnapBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = SnapBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::Snap, "firefox"),
            name: "Firefox".into(),
            installed: Version::parse("140.0"),
            available: Version::parse(""),
            size_bytes: None,
            expected_sha256: None,
        };
        let err = backend.apply(&candidate).await.unwrap_err();
        assert!(matches!(err, Error::Verification { .. }));
    }
}
