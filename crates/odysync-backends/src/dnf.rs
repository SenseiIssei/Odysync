//! DNF backend for Fedora, RHEL, Rocky, and AlmaLinux.
//!
//! DNF verifies package signatures at the repository level, so integrity is
//! handled below us. We pin the exact target version with `pkg-version` syntax
//! to prevent DNF from resolving "latest" on its own.
//!
//! Commands used:
//!   - `dnf check-update --refresh -q` — list available updates (exit 100 = updates exist)
//!   - `dnf install -y --best pkg-version` — install a specific version
//!   - `rpm -q --qf '%{VERSION}-%{RELEASE}' pkg` — read installed version

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

const SCAN_TIMEOUT: Duration = Duration::from_secs(300);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(45 * 60);
const QUERY_TIMEOUT: Duration = Duration::from_secs(60);

pub struct DnfBackend;

impl DnfBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DnfBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for DnfBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Dnf
    }

    fn display_name(&self) -> &str {
        "DNF"
    }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && proc::exists("dnf", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let out = proc::run(
            "dnf",
            &["check-update", "--refresh", "-q"],
            SCAN_TIMEOUT,
        )
        .await?;

        // Exit 100 = updates available, 0 = no updates.
        if out.code != 0 && out.code != 100 {
            return Err(Error::CommandFailed {
                command: "dnf check-update".into(),
                code: out.code,
                stderr: if out.stderr.trim().is_empty() {
                    out.stdout
                } else {
                    out.stderr
                },
            });
        }

        Ok(parse_dnf_check_update(&out.stdout))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }

        let spec = format!("{}-{}", candidate.id.native, candidate.available.raw());
        let out = proc::run(
            "dnf",
            &["install", "-y", "--best", &spec],
            INSTALL_TIMEOUT,
        )
        .await?;

        if out.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                command: format!("dnf install {spec}"),
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
            "rpm",
            &["-q", "--qf", "%{VERSION}-%{RELEASE}", &candidate.id.native],
            QUERY_TIMEOUT,
        )
        .await?;

        if !out.success() || out.stdout.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(out.stdout.trim().to_string()))
    }
}

/// Parse `dnf check-update -q` output.
///
/// Quiet output is one package per line: `package-name.arch    version-release    repo`
fn parse_dnf_check_update(stdout: &str) -> Vec<UpdateCandidate> {
    stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            if line.contains("Last metadata expiration") {
                return None;
            }
            // Skip summary lines like "N updates can be installed" or similar.
            if line.starts_with("Last metadata") || (line.contains("update") && !line.contains('.')) {
                return None;
            }

            let mut parts = line.split_whitespace();
            let pkg_arch = parts.next()?;
            let version_release = parts.next()?;

            let name = pkg_arch.rsplit_once('.').map(|(n, _)| n).unwrap_or(pkg_arch);

            Some(UpdateCandidate {
                id: PackageId::new(BackendKind::Dnf, name),
                name: name.to_string(),
                installed: Version::parse(""),
                available: Version::parse(version_release),
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
    fn parses_dnf_check_update_output() {
        let output = "\
firefox.x86_64            141.0-1.fc40       updates
kernel.x86_64             6.8.0-50.fc40      updates
";
        let candidates = parse_dnf_check_update(output);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].id.native, "firefox");
        assert_eq!(candidates[0].available.raw(), "141.0-1.fc40");
        assert_eq!(candidates[1].id.native, "kernel");
    }

    #[test]
    fn skips_non_package_lines() {
        let output = "\
Last metadata expiration check: 0:01:23 ago
# comment
firefox.x86_64            141.0-1.fc40       updates
";
        let candidates = parse_dnf_check_update(output);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id.native, "firefox");
    }

    #[test]
    fn empty_output_yields_empty_vec() {
        assert!(parse_dnf_check_update("").is_empty());
    }

    #[test]
    fn backend_kind_is_correct() {
        let b = DnfBackend::new();
        assert_eq!(b.kind(), BackendKind::Dnf);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = DnfBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = DnfBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::Dnf, "firefox"),
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
