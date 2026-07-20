//! Flatpak backend.
//!
//! Flatpak verifies its own OSTree commits against the remote's GPG key, so
//! integrity is handled below us. We use the machine-readable `--columns`
//! output rather than the human table, which avoids the localisation problem
//! entirely.

use std::time::Duration;

use async_trait::async_trait;
use sensei_core::backend::Backend;
use sensei_core::error::{Error, Result};
use sensei_core::model::{BackendKind, PackageId, UpdateCandidate};
use sensei_core::proc;
use sensei_core::version::Version;

const SCAN_TIMEOUT: Duration = Duration::from_secs(240);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(45 * 60);
const QUERY_TIMEOUT: Duration = Duration::from_secs(60);

pub struct FlatpakBackend;

impl FlatpakBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FlatpakBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for FlatpakBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Flatpak
    }

    fn display_name(&self) -> &str {
        "Flatpak"
    }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && proc::exists("flatpak", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        // `remote-ls --updates` lists what the remotes offer that is newer
        // than what is installed.
        let out = proc::run(
            "flatpak",
            &[
                "remote-ls",
                "--updates",
                "--app",
                "--columns=application,version",
            ],
            SCAN_TIMEOUT,
        )
        .await?;

        if !out.success() {
            return Err(Error::CommandFailed {
                command: "flatpak remote-ls --updates".into(),
                code: out.code,
                stderr: out.stderr,
            });
        }

        let installed = self.installed_map().await;
        Ok(parse_columns(&out.stdout)
            .into_iter()
            .map(|(app, version)| UpdateCandidate {
                id: PackageId::new(BackendKind::Flatpak, app.clone()),
                name: app.clone(),
                installed: Version::parse(
                    installed
                        .iter()
                        .find(|(a, _)| *a == app)
                        .map(|(_, v)| v.as_str())
                        .unwrap_or(""),
                ),
                available: Version::parse(&version),
                size_bytes: None,
                expected_sha256: None,
            })
            .collect())
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        let out = proc::run(
            "flatpak",
            &["update", "-y", "--noninteractive", &candidate.id.native],
            INSTALL_TIMEOUT,
        )
        .await?;

        if out.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                command: format!("flatpak update {}", candidate.id.native),
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
            "flatpak",
            &["list", "--app", "--columns=application,version"],
            QUERY_TIMEOUT,
        )
        .await?;

        Ok(parse_columns(&out.stdout)
            .into_iter()
            .find(|(app, _)| app.eq_ignore_ascii_case(&candidate.id.native))
            .map(|(_, v)| v))
    }
}

impl FlatpakBackend {
    async fn installed_map(&self) -> Vec<(String, String)> {
        match proc::run(
            "flatpak",
            &["list", "--app", "--columns=application,version"],
            QUERY_TIMEOUT,
        )
        .await
        {
            Ok(out) if out.success() => parse_columns(&out.stdout),
            _ => Vec::new(),
        }
    }
}

/// Parse flatpak's `--columns` output: tab-separated, no header.
fn parse_columns(stdout: &str) -> Vec<(String, String)> {
    stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim_end();
            if line.trim().is_empty() {
                return None;
            }
            // Tab-separated by contract, but fall back to whitespace in case a
            // version column is empty.
            let mut parts = line.split('\t');
            let app = parts.next()?.trim().to_string();
            let version = parts.next().unwrap_or("").trim().to_string();
            if app.is_empty() {
                None
            } else {
                Some((app, version))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tab_separated_columns() {
        let out = "org.mozilla.firefox\t141.0\norg.gimp.GIMP\t2.10.38\n";
        let rows = parse_columns(out);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], ("org.mozilla.firefox".into(), "141.0".into()));
        assert_eq!(rows[1].1, "2.10.38");
    }

    #[test]
    fn an_app_with_no_version_still_parses() {
        // Flatpak leaves the version blank for some runtimes; policy rejects
        // it later, but the scan must not lose the row or error.
        let rows = parse_columns("org.example.App\t\n");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "");
        assert!(!Version::parse(&rows[0].1).is_known());
    }

    #[test]
    fn blank_lines_are_skipped() {
        assert!(parse_columns("\n\n   \n").is_empty());
    }
}
