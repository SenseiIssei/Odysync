//! Zypper backend for SUSE/openSUSE.
//!
//! Zypper verifies package signatures at the repository level. We pin versions
//! using zypper's built-in update mechanism.
//!
//! Commands used:
//!   - `zypper list-updates --type all` — list available updates
//!   - `zypper update -y <pkg>` — update a specific package
//!   - `rpm -q --qf '%{VERSION}-%{RELEASE}' <pkg>` — read installed version

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

pub struct ZypperBackend;

impl ZypperBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ZypperBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for ZypperBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Zypper
    }

    fn display_name(&self) -> &str {
        "Zypper"
    }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && proc::exists("zypper", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let out = proc::run(
            "zypper",
            &["--no-refresh", "list-updates", "--type", "all"],
            SCAN_TIMEOUT,
        )
        .await?;

        if !out.success() {
            return Err(Error::CommandFailed {
                command: "zypper list-updates".into(),
                code: out.code,
                stderr: out.stderr,
            });
        }

        Ok(parse_zypper_list_updates(&out.stdout))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }

        let out = proc::run(
            "zypper",
            &["update", "-y", &candidate.id.native],
            INSTALL_TIMEOUT,
        )
        .await?;

        if out.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                command: format!("zypper update {}", candidate.id.native),
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

/// Parse `zypper list-updates --type all` output.
///
/// Format (table with header):
/// ```text
/// S | Repository         | Name       | Current Version | Available Version | Arch
/// --|--------------------|------------|-----------------|-------------------|-------
/// v | Update repository  | firefox    | 140.0-1.1       | 141.0-1.1         | x86_64
/// ```
fn parse_zypper_list_updates(stdout: &str) -> Vec<UpdateCandidate> {
    stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('-') || line.starts_with("S |") {
                return None;
            }

            // Split by '|' and collect trimmed fields.
            let fields: Vec<&str> = line.split('|').map(|f| f.trim()).collect();
            if fields.len() < 6 {
                return None;
            }

            // fields[0] = status (v, !, etc.), fields[2] = name, fields[3] = current, fields[4] = available
            let name = fields[2];
            let current_version = fields[3];
            let available_version = fields[4];

            if name.is_empty() {
                return None;
            }

            Some(UpdateCandidate {
                id: PackageId::new(BackendKind::Zypper, name),
                name: name.to_string(),
                installed: Version::parse(current_version),
                available: Version::parse(available_version),
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
    fn parses_zypper_list_updates() {
        let output = "\
S | Repository         | Name       | Current Version | Available Version | Arch
--|--------------------|------------|-----------------|-------------------|-------
v | Update repository  | firefox    | 140.0-1.1       | 141.0-1.1         | x86_64
v | Update repository  | kernel     | 6.8.0-49.1      | 6.8.0-50.1        | x86_64
";
        let candidates = parse_zypper_list_updates(output);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].id.native, "firefox");
        assert_eq!(candidates[0].installed.raw(), "140.0-1.1");
        assert_eq!(candidates[0].available.raw(), "141.0-1.1");
        assert_eq!(candidates[1].id.native, "kernel");
    }

    #[test]
    fn skips_header_and_separator() {
        let output = "\
S | Repository | Name | Current | Available | Arch
--|------------|------|---------|-----------|------
";
        assert!(parse_zypper_list_updates(output).is_empty());
    }

    #[test]
    fn empty_output_yields_empty_vec() {
        assert!(parse_zypper_list_updates("").is_empty());
    }

    #[test]
    fn backend_kind_is_correct() {
        let b = ZypperBackend::new();
        assert_eq!(b.kind(), BackendKind::Zypper);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = ZypperBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = ZypperBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::Zypper, "firefox"),
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
