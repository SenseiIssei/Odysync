//! Nix backend for Linux and macOS.
//!
//! Nix is a declarative package manager that can run in multi-user or
//! single-user mode. We use `nix profile list` to enumerate installed packages
//! and `nix profile upgrade` to apply updates.
//!
//! Commands used:
//!   - `nix profile list` — list installed packages
//!   - `nix profile upgrade <index>` — upgrade a specific profile entry
//!   - `nix profile list` — read back installed version
//!
//! Reference: https://nixos.org/manual/nix/stable/command-ref/new-cli/nix3-profile.html

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

const SCAN_TIMEOUT: Duration = Duration::from_secs(120);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);
const QUERY_TIMEOUT: Duration = Duration::from_secs(60);

pub struct NixBackend;

impl NixBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NixBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for NixBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Nix
    }

    fn display_name(&self) -> &str {
        "Nix"
    }

    async fn is_available(&self) -> bool {
        (cfg!(target_os = "linux") || cfg!(target_os = "macos"))
            && proc::exists("nix", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        // `nix profile list` outputs structured data about installed packages.
        // We parse it to find packages and their versions, then compare with
        // what's available via `nix flake metadata`.
        //
        // Since Nix's new CLI doesn't have a direct "list outdated" command,
        // we list installed packages and report them with unknown available
        // versions. The user can then use `nix profile upgrade` to update all.
        let out = proc::run("nix", &["profile", "list"], SCAN_TIMEOUT).await?;

        if !out.success() {
            return Err(Error::CommandFailed {
                command: "nix profile list".into(),
                code: out.code,
                stderr: out.stderr,
            });
        }

        Ok(parse_nix_profile_list(&out.stdout))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }

        // Nix profile upgrade takes an index, not a package name.
        // We use the native field which stores the index.
        let out = proc::run(
            "nix",
            &["profile", "upgrade", &candidate.id.native],
            INSTALL_TIMEOUT,
        )
        .await?;

        if out.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                command: format!("nix profile upgrade {}", candidate.id.native),
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
        let out = proc::run("nix", &["profile", "list"], QUERY_TIMEOUT).await?;

        if !out.success() {
            return Ok(None);
        }

        // Find the entry matching the candidate's native id (index).
        for entry in parse_nix_profile_list(&out.stdout) {
            if entry.id.native == candidate.id.native {
                return Ok(Some(entry.installed.raw().to_string()));
            }
        }

        Ok(None)
    }
}

/// Parse `nix profile list` output.
///
/// Output format (simplified):
/// ```text
/// 0 flake:nixpkgs#firefox github:NixOS/nixpkgs/abc123 firefox 141.0
/// 1 flake:nixpkgs#git github:NixOS/nixpkgs/def456 git 2.43.0
/// ```
fn parse_nix_profile_list(stdout: &str) -> Vec<UpdateCandidate> {
    stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                return None;
            }

            // First field is the index, second is the flake ref.
            let index = parts[0];
            let flake_ref = parts[1];

            // Extract package name from flake ref (after #).
            let name = flake_ref
                .rsplit_once('#')
                .map(|(_, n)| n)
                .unwrap_or(flake_ref);

            // Try to find version in remaining parts — usually the last field.
            let version = parts.last().copied().unwrap_or("");

            // Skip if version looks like a hash (40+ hex chars).
            let version = if version.len() >= 40 && version.chars().all(|c| c.is_ascii_hexdigit()) {
                ""
            } else {
                version
            };

            Some(UpdateCandidate {
                id: PackageId::new(BackendKind::Nix, index),
                name: name.to_string(),
                installed: Version::parse(version),
                available: Version::parse(""), // Nix doesn't expose available version directly
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
    fn parses_nix_profile_list() {
        let output = "\
0 flake:nixpkgs#firefox github:NixOS/nixpkgs/abc123 firefox 141.0
1 flake:nixpkgs#git github:NixOS/nixpkgs/def456 git 2.43.0
";
        let candidates = parse_nix_profile_list(output);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].id.native, "0");
        assert_eq!(candidates[0].name, "firefox");
        assert_eq!(candidates[0].installed.raw(), "141.0");
        assert_eq!(candidates[1].id.native, "1");
        assert_eq!(candidates[1].name, "git");
    }

    #[test]
    fn skips_hash_like_versions() {
        let output = "0 flake:nixpkgs#firefox abc123def456abc123def456abc123def456abc123\n";
        let candidates = parse_nix_profile_list(output);
        assert_eq!(candidates.len(), 1);
        assert!(!candidates[0].installed.is_known());
    }

    #[test]
    fn empty_output_yields_empty_vec() {
        assert!(parse_nix_profile_list("").is_empty());
    }

    #[test]
    fn backend_kind_is_correct() {
        let b = NixBackend::new();
        assert_eq!(b.kind(), BackendKind::Nix);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = NixBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = NixBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::Nix, "0"),
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
