//! Pacman backend for Arch Linux and derivatives.
//!
//! Pacman verifies package signatures via its Pacman-key infrastructure, so
//! integrity is handled below us. Version pinning uses `pkg=version` syntax.
//!
//! Commands used:
//!   - `checkupdates` — list available updates (uses a temp sync DB)
//!   - `pacman -S --noconfirm --needed pkg=version` — install a specific version
//!   - `pacman -Q pkg` — read installed version

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

const SCAN_TIMEOUT: Duration = Duration::from_secs(300);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(45 * 60);
const QUERY_TIMEOUT: Duration = Duration::from_secs(30);

pub struct PacmanBackend;

impl PacmanBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PacmanBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for PacmanBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Pacman
    }

    fn display_name(&self) -> &str {
        "Pacman"
    }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && proc::exists("pacman", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        // `checkupdates` uses a temporary sync database so it doesn't require
        // root or interfere with the system's pacman database.
        let out = proc::run("checkupdates", &[] as &[&str], SCAN_TIMEOUT).await;

        // checkupdates exits 0 with updates on stdout, or 2 when no updates.
        let stdout = match out {
            Ok(o) if o.code == 0 || o.code == 2 => o.stdout,
            Ok(o) => {
                return Err(Error::CommandFailed {
                    command: "checkupdates".into(),
                    code: o.code,
                    stderr: o.stderr,
                });
            }
            Err(e) => {
                // checkupdates may not be installed; fall back to pacman -Syu --print
                tracing::warn!(error = %e, "checkupdates not available, falling back to pacman");
                return self.scan_via_pacman().await;
            }
        };

        Ok(parse_checkupdates(&stdout))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }

        // Pin the exact version with pkg=version syntax.
        let spec = format!("{}={}", candidate.id.native, candidate.available.raw());
        let out = proc::run(
            "pacman",
            &["-S", "--noconfirm", "--needed", &spec],
            INSTALL_TIMEOUT,
        )
        .await?;

        if out.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                command: format!("pacman -S {spec}"),
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
        let out = proc::run(
            "pacman",
            &["-Q", &candidate.id.native],
            QUERY_TIMEOUT,
        )
        .await?;

        if !out.success() {
            return Ok(None);
        }

        // Output: "name  version" (space-separated)
        let line = out.stdout.lines().next().unwrap_or("").trim();
        let version = line.split_whitespace().nth(1).unwrap_or("");
        if version.is_empty() {
            Ok(None)
        } else {
            Ok(Some(version.to_string()))
        }
    }
}

impl PacmanBackend {
    /// Fallback scan using `pacman -Qu` (less reliable than checkupdates).
    async fn scan_via_pacman(&self) -> Result<Vec<UpdateCandidate>> {
        let out = proc::run("pacman", &["-Qu"], SCAN_TIMEOUT).await?;

        if !out.success() {
            return Ok(Vec::new());
        }

        Ok(parse_pacman_qu(&out.stdout))
    }
}

/// Parse `checkupdates` output.
///
/// Format: `package-name  old_version -> new_version`
fn parse_checkupdates(stdout: &str) -> Vec<UpdateCandidate> {
    stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }

            // Format: "package  old_version -> new_version"
            let mut parts = line.split_whitespace();
            let name = parts.next()?;
            let old_version = parts.next()?;
            let arrow = parts.next()?;
            if arrow != "->" {
                return None;
            }
            let new_version = parts.next()?;

            Some(UpdateCandidate {
                id: PackageId::new(BackendKind::Pacman, name),
                name: name.to_string(),
                installed: Version::parse(old_version),
                available: Version::parse(new_version),
                size_bytes: None,
                expected_sha256: None,
            })
        })
        .collect()
}

/// Parse `pacman -Qu` output (fallback).
///
/// Format: `package-name  old_version -> new_version` (same as checkupdates)
fn parse_pacman_qu(stdout: &str) -> Vec<UpdateCandidate> {
    parse_checkupdates(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_checkupdates_output() {
        let output = "\
firefox  140.0-1 -> 141.0-1
linux  6.8.0-49 -> 6.8.0-50
";
        let candidates = parse_checkupdates(output);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].id.native, "firefox");
        assert_eq!(candidates[0].installed.raw(), "140.0-1");
        assert_eq!(candidates[0].available.raw(), "141.0-1");
        assert_eq!(candidates[1].id.native, "linux");
    }

    #[test]
    fn empty_output_yields_empty_vec() {
        assert!(parse_checkupdates("").is_empty());
    }

    #[test]
    fn skips_malformed_lines() {
        let output = "firefox  140.0-1 -> 141.0-1\nbad line no arrow\n";
        let candidates = parse_checkupdates(output);
        assert_eq!(candidates.len(), 1);
    }

    #[test]
    fn backend_kind_is_correct() {
        let b = PacmanBackend::new();
        assert_eq!(b.kind(), BackendKind::Pacman);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = PacmanBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = PacmanBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::Pacman, "firefox"),
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
