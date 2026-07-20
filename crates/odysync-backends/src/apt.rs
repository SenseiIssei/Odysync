//! APT backend for Debian, Ubuntu and derivatives.
//!
//! APT is the safest of the three platforms by construction: packages are
//! signed at the repository level and `apt-get` verifies those signatures
//! itself, so integrity does not depend on us. What we add is exact version
//! pinning (`apt-get install pkg=version`), which keeps the install identical
//! to what the user approved in the plan even if the mirror moves underneath.

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

pub struct AptBackend;

impl AptBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AptBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for AptBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Apt
    }

    fn display_name(&self) -> &str {
        "APT"
    }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && proc::exists("apt-get", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        // Refreshing the index needs root; without it we still scan, just
        // against whatever the last `apt update` left behind.
        if odysync_core::platform::is_elevated() {
            let _ = proc::run("apt-get", &["update", "-qq"], SCAN_TIMEOUT).await;
        }

        let out = proc::run("apt", &["list", "--upgradable"], SCAN_TIMEOUT).await?;
        if !out.success() {
            return Err(Error::CommandFailed {
                command: "apt list --upgradable".into(),
                code: out.code,
                stderr: out.stderr,
            });
        }

        Ok(parse_upgradable(&out.stdout))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }

        // Pin the exact version, and refuse anything that would remove other
        // packages to satisfy the upgrade. `--no-remove` turns a destructive
        // dependency resolution into a clean failure.
        let spec = format!("{}={}", candidate.id.native, candidate.available.raw());
        let out = proc::run(
            "apt-get",
            &[
                "install",
                "--only-upgrade",
                "--no-remove",
                "-y",
                "-o",
                "Dpkg::Options::=--force-confold",
                &spec,
            ],
            INSTALL_TIMEOUT,
        )
        .await?;

        if out.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                command: format!("apt-get install {spec}"),
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
            "dpkg-query",
            &["-W", "-f=${Version}", &candidate.id.native],
            QUERY_TIMEOUT,
        )
        .await?;

        if !out.success() || out.stdout.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(out.stdout.trim().to_string()))
    }
}

/// Parse `apt list --upgradable`.
///
/// Each line looks like:
/// `firefox/noble-updates 141.0+build1 amd64 [upgradable from: 140.0+build2]`
fn parse_upgradable(stdout: &str) -> Vec<UpdateCandidate> {
    let mut out = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        // The first line is "Listing..." and there may be warnings.
        if line.is_empty() || !line.contains('/') || !line.contains("upgradable from:") {
            continue;
        }

        let Some((name_part, rest)) = line.split_once('/') else {
            continue;
        };
        let name = name_part.trim();
        if name.is_empty() {
            continue;
        }

        // rest: "noble-updates 141.0+build1 amd64 [upgradable from: 140.0+build2]"
        let mut fields = rest.split_whitespace();
        let _suite = fields.next();
        let Some(available) = fields.next() else {
            continue;
        };

        let installed = line
            .split_once("upgradable from:")
            .map(|(_, tail)| tail.trim().trim_end_matches(']').trim())
            .unwrap_or("");

        out.push(UpdateCandidate {
            id: PackageId::new(BackendKind::Apt, name),
            name: name.to_string(),
            installed: Version::parse(installed),
            available: Version::parse(available),
            size_bytes: None,
            expected_sha256: None,
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_standard_upgradable_line() {
        let out = "Listing...\n\
firefox/noble-updates 141.0+build1 amd64 [upgradable from: 140.0+build2]\n";
        let rows = parse_upgradable(out);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id.native, "firefox");
        assert_eq!(rows[0].available.raw(), "141.0+build1");
        assert_eq!(rows[0].installed.raw(), "140.0+build2");
    }

    #[test]
    fn the_listing_header_and_blank_lines_are_ignored() {
        assert!(parse_upgradable("Listing...\n\n").is_empty());
    }

    #[test]
    fn lines_without_an_upgrade_marker_are_ignored() {
        // Plain `apt list` output must not be mistaken for upgradable packages.
        let out = "Listing...\nfirefox/noble,now 140.0 amd64 [installed]\n";
        assert!(parse_upgradable(out).is_empty());
    }

    #[test]
    fn parses_several_packages() {
        let out = "Listing...\n\
curl/noble-updates 8.5.0-2 amd64 [upgradable from: 8.4.0-1]\n\
vim/noble-updates 9.1.0-1 amd64 [upgradable from: 9.0.2-1]\n";
        let rows = parse_upgradable(out);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].id.native, "vim");
    }

    #[test]
    fn epoch_versions_survive_parsing() {
        // Debian epochs contain a colon and must not be truncated.
        let out = "Listing...\n\
tzdata/noble 2:2024a-1 all [upgradable from: 2:2023d-1]\n";
        let rows = parse_upgradable(out);
        assert_eq!(rows[0].available.raw(), "2:2024a-1");
        assert_eq!(rows[0].installed.raw(), "2:2023d-1");
    }

    #[tokio::test]
    async fn apply_refuses_without_an_exact_target_version() {
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::Apt, "firefox"),
            name: "firefox".into(),
            installed: Version::parse("1.0"),
            available: Version::parse(""),
            size_bytes: None,
            expected_sha256: None,
        };
        assert!(matches!(
            AptBackend::new().apply(&candidate).await.unwrap_err(),
            Error::Verification { .. }
        ));
    }
}
