//! Chocolatey backend for Windows.
//!
//! Chocolatey is a Windows package manager that verifies package checksums
//! and signatures. We use `--limit-output` for machine-readable output.
//!
//! Commands used:
//!   - `choco outdated --limit-output` — list outdated packages
//!   - `choco install <pkg> --version <ver> -y --no-progress` — install specific version
//!   - `choco list --local-only --limit-output <pkg>` — read installed version

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, InstalledPackage, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

const SCAN_TIMEOUT: Duration = Duration::from_secs(120);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);
const QUERY_TIMEOUT: Duration = Duration::from_secs(30);

pub struct ChocolateyBackend;

impl ChocolateyBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ChocolateyBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for ChocolateyBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Chocolatey
    }

    fn display_name(&self) -> &str {
        "Chocolatey"
    }

    async fn is_available(&self) -> bool {
        cfg!(windows) && proc::exists("choco", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let out = proc::run("choco", &["outdated", "--limit-output"], SCAN_TIMEOUT).await?;

        if !out.success() {
            return Err(Error::CommandFailed {
                command: "choco outdated".into(),
                code: out.code,
                stderr: out.stderr,
            });
        }

        Ok(parse_choco_outdated(&out.stdout))
    }

    async fn list_installed(&self) -> Result<Vec<InstalledPackage>> {
        let out = proc::run(
            "choco",
            &["list", "--local-only", "--limit-output"],
            SCAN_TIMEOUT,
        )
        .await?;

        if !out.success() {
            return Err(Error::CommandFailed {
                command: "choco list --local-only".into(),
                code: out.code,
                stderr: out.stderr,
            });
        }

        Ok(parse_choco_list(&out.stdout))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }

        let out = proc::run(
            "choco",
            &[
                "install",
                &candidate.id.native,
                "--version",
                candidate.available.raw(),
                "-y",
                "--no-progress",
            ],
            INSTALL_TIMEOUT,
        )
        .await?;

        if out.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                command: format!(
                    "choco install {} {}",
                    candidate.id.native,
                    candidate.available.raw()
                ),
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
            "choco",
            &[
                "list",
                "--local-only",
                "--limit-output",
                &candidate.id.native,
            ],
            QUERY_TIMEOUT,
        )
        .await?;

        if !out.success() {
            return Ok(None);
        }

        // Output: "package_name|version"
        let line = out.stdout.lines().find(|l| {
            let l = l.trim();
            !l.is_empty()
        });
        let version = line
            .and_then(|l| l.split('|').nth(1))
            .map(|v| v.trim().to_string());

        Ok(version.filter(|v| !v.is_empty()))
    }
}

/// Parse `choco outdated --limit-output` output.
///
/// Format: `package_name|current_version|available_version|pinned?(true/false)`
fn parse_choco_outdated(stdout: &str) -> Vec<UpdateCandidate> {
    stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }

            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() < 3 {
                return None;
            }

            let name = parts[0].trim();
            let current_version = parts[1].trim();
            let available_version = parts[2].trim();

            if name.is_empty() {
                return None;
            }

            Some(UpdateCandidate {
                id: PackageId::new(BackendKind::Chocolatey, name),
                name: name.to_string(),
                installed: Version::parse(current_version),
                available: Version::parse(available_version),
                size_bytes: None,
                expected_sha256: None,
            })
        })
        .collect()
}

/// Parse `choco list --local-only --limit-output` output.
///
/// Format: `package_name|version`, one per line. Lines without a pipe are
/// banner or summary text ("3 packages installed.") and are skipped rather
/// than guessed at.
fn parse_choco_list(stdout: &str) -> Vec<InstalledPackage> {
    stdout
        .lines()
        .filter_map(|line| {
            let (name, version) = line.trim().split_once('|')?;
            let name = name.trim();
            let version = version.split('|').next().unwrap_or("").trim();
            if name.is_empty() {
                return None;
            }
            Some(InstalledPackage {
                id: PackageId::new(BackendKind::Chocolatey, name),
                name: name.to_string(),
                version: version.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_choco_list() {
        let output = "\
chocolatey|2.2.2
firefox|141.0
7zip|24.09
";
        let installed = parse_choco_list(output);
        assert_eq!(installed.len(), 3);
        assert_eq!(installed[0].id.native, "chocolatey");
        assert_eq!(installed[0].version, "2.2.2");
        assert_eq!(installed[1].name, "firefox");
        assert_eq!(installed[2].version, "24.09");
    }

    #[test]
    fn choco_list_skips_lines_without_a_pipe() {
        let output = "firefox|141.0\n3 packages installed.\n\n";
        let installed = parse_choco_list(output);
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].id.native, "firefox");
    }

    #[test]
    fn choco_list_empty_output_yields_empty_vec() {
        assert!(parse_choco_list("").is_empty());
    }

    #[test]
    fn parses_choco_outdated() {
        let output = "\
firefox|140.0|141.0|false
7zip|23.01|24.07|false
";
        let candidates = parse_choco_outdated(output);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].id.native, "firefox");
        assert_eq!(candidates[0].installed.raw(), "140.0");
        assert_eq!(candidates[0].available.raw(), "141.0");
        assert_eq!(candidates[1].id.native, "7zip");
    }

    #[test]
    fn skips_malformed_lines() {
        let output = "firefox|140.0|141.0|false\nbad line\n";
        let candidates = parse_choco_outdated(output);
        assert_eq!(candidates.len(), 1);
    }

    #[test]
    fn empty_output_yields_empty_vec() {
        assert!(parse_choco_outdated("").is_empty());
    }

    #[test]
    fn backend_kind_is_correct() {
        let b = ChocolateyBackend::new();
        assert_eq!(b.kind(), BackendKind::Chocolatey);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = ChocolateyBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = ChocolateyBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::Chocolatey, "firefox"),
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
