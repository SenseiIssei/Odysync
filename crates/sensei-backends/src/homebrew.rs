//! Homebrew backend for macOS (and Linuxbrew).
//!
//! Homebrew has real machine-readable output, so unlike winget there is no
//! table parsing here. The safety story differs too: `brew upgrade <formula>`
//! always moves to the newest version in the tapped formula, and Homebrew has
//! no per-invocation version pin. We therefore verify convergence afterwards
//! and let the runner flag any package that landed somewhere unexpected.

use std::time::Duration;

use async_trait::async_trait;
use sensei_core::backend::Backend;
use sensei_core::error::{Error, Result};
use sensei_core::model::{BackendKind, PackageId, UpdateCandidate};
use sensei_core::proc;
use sensei_core::version::Version;
use serde::Deserialize;

const SCAN_TIMEOUT: Duration = Duration::from_secs(300);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(45 * 60);
const QUERY_TIMEOUT: Duration = Duration::from_secs(120);

pub struct HomebrewBackend;

#[derive(Debug, Deserialize)]
struct Outdated {
    #[serde(default)]
    formulae: Vec<OutdatedItem>,
    #[serde(default)]
    casks: Vec<OutdatedItem>,
}

#[derive(Debug, Deserialize)]
struct OutdatedItem {
    name: String,
    #[serde(default)]
    installed_versions: Vec<String>,
    #[serde(default)]
    current_version: Option<String>,
}

impl HomebrewBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for HomebrewBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for HomebrewBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Homebrew
    }

    fn display_name(&self) -> &str {
        "Homebrew"
    }

    async fn is_available(&self) -> bool {
        !cfg!(windows) && proc::exists("brew", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        // `brew update` refreshes the formula index; without it `outdated` is
        // answered from a stale local copy and misses everything published
        // since the last run.
        let _ = proc::run("brew", &["update", "--quiet"], SCAN_TIMEOUT).await;

        let out = proc::run(
            "brew",
            &["outdated", "--json=v2", "--greedy-auto-updates"],
            SCAN_TIMEOUT,
        )
        .await?;

        if !out.success() {
            return Err(Error::CommandFailed {
                command: "brew outdated".into(),
                code: out.code,
                stderr: out.stderr,
            });
        }

        let parsed: Outdated = serde_json::from_str(out.stdout.trim())
            .map_err(|e| Error::parse("brew outdated --json=v2", e.to_string()))?;

        Ok(parsed
            .formulae
            .into_iter()
            .chain(parsed.casks)
            .map(|item| UpdateCandidate {
                id: PackageId::new(BackendKind::Homebrew, item.name.clone()),
                name: item.name,
                // Homebrew can report several installed versions side by side;
                // the last is the one currently linked.
                installed: Version::parse(
                    item.installed_versions
                        .last()
                        .map(String::as_str)
                        .unwrap_or(""),
                ),
                available: Version::parse(item.current_version.as_deref().unwrap_or("")),
                size_bytes: None,
                expected_sha256: None,
            })
            .collect())
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        // Homebrew refuses to run as root and can corrupt the prefix's
        // ownership if forced; the policy layer already blocks this, but the
        // backend refuses independently so no caller can bypass it.
        if sensei_core::platform::is_elevated() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "Homebrew must not be run as root".into(),
            });
        }

        let out = proc::run(
            "brew",
            &["upgrade", "--quiet", &candidate.id.native],
            INSTALL_TIMEOUT,
        )
        .await?;

        if out.success() {
            Ok(())
        } else {
            // No reinstall fallback here either.
            Err(Error::CommandFailed {
                command: format!("brew upgrade {}", candidate.id.native),
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
            "brew",
            &["list", "--versions", &candidate.id.native],
            QUERY_TIMEOUT,
        )
        .await?;

        if !out.success() {
            return Ok(None);
        }

        // Output is "name 1.2.3" or "name 1.2.3 1.2.4" when several are kept.
        Ok(parse_list_versions(&out.stdout))
    }
}

/// Extract the newest version from `brew list --versions` output.
fn parse_list_versions(stdout: &str) -> Option<String> {
    let line = stdout.lines().find(|l| !l.trim().is_empty())?;
    let mut parts = line.split_whitespace();
    let _name = parts.next()?;
    // Later entries are newer in brew's ordering.
    parts.last().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_single_installed_version() {
        assert_eq!(parse_list_versions("wget 1.21.4\n"), Some("1.21.4".into()));
    }

    #[test]
    fn picks_the_newest_when_several_are_kept() {
        assert_eq!(
            parse_list_versions("openssl 3.1.0 3.2.1\n"),
            Some("3.2.1".into())
        );
    }

    #[test]
    fn empty_output_yields_none() {
        assert_eq!(parse_list_versions(""), None);
        assert_eq!(parse_list_versions("\n  \n"), None);
    }

    #[test]
    fn a_name_with_no_version_yields_none() {
        assert_eq!(parse_list_versions("somepkg\n"), None);
    }

    #[test]
    fn parses_formulae_and_casks_into_one_list() {
        let json = r#"{
            "formulae": [
                {"name": "wget", "installed_versions": ["1.21.3"], "current_version": "1.21.4"}
            ],
            "casks": [
                {"name": "firefox", "installed_versions": ["140.0"], "current_version": "141.0"}
            ]
        }"#;
        let parsed: Outdated = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.formulae.len(), 1);
        assert_eq!(parsed.casks.len(), 1);
        assert_eq!(parsed.casks[0].current_version.as_deref(), Some("141.0"));
    }

    #[test]
    fn missing_version_fields_do_not_fail_parsing() {
        // A cask with no known current version must still parse, so policy can
        // reject it with a clear reason instead of the scan erroring out.
        let json = r#"{"formulae": [{"name": "mystery"}], "casks": []}"#;
        let parsed: Outdated = serde_json::from_str(json).unwrap();
        assert!(parsed.formulae[0].current_version.is_none());
        assert!(!Version::parse("").is_known());
    }
}
