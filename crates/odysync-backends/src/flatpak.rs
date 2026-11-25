//! Flatpak backend.
//!
//! Flatpak verifies its own OSTree commits against the remote's GPG key, so
//! integrity is handled below us. We use the machine-readable `--columns`
//! output rather than the human table, which avoids the localisation problem
//! entirely.
//!
//! `flatpak update` takes no version. What it would install is therefore read
//! again right before, and a target that moved is refused.

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::Result;
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

use crate::guard;

const SCAN_TIMEOUT: Duration = Duration::from_secs(240);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(45 * 60);
const QUERY_TIMEOUT: Duration = Duration::from_secs(60);

pub struct FlatpakBackend;

impl FlatpakBackend {
    pub fn new() -> Self {
        Self
    }

    /// What the remotes offer that is newer than what is installed.
    async fn offered() -> Result<Vec<(String, String)>> {
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
            return Err(guard::failed("flatpak remote-ls --updates", out));
        }
        Ok(parse_columns(&out.stdout))
    }

    async fn installed() -> Result<Vec<(String, String)>> {
        let out = proc::run(
            "flatpak",
            &["list", "--app", "--columns=application,version"],
            QUERY_TIMEOUT,
        )
        .await?;
        if !out.success() {
            return Err(guard::failed("flatpak list", out));
        }
        Ok(parse_columns(&out.stdout))
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
        let offered = Self::offered().await?;
        if offered.is_empty() {
            return Ok(Vec::new());
        }
        Ok(join(&offered, &Self::installed().await?))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        guard::precheck(candidate)?;
        let offered = Self::offered().await?;
        guard::same_target(candidate, only_version(&offered, &candidate.id.native))?;
        guard::still_older(
            candidate,
            self.installed_version(candidate).await?.as_deref(),
        )?;

        let out = proc::run(
            "flatpak",
            &[
                "update",
                "-y",
                "--noninteractive",
                "--app",
                &candidate.id.native,
            ],
            INSTALL_TIMEOUT,
        )
        .await?;
        if out.success() {
            Ok(())
        } else {
            Err(guard::failed(
                format!("flatpak update {}", candidate.id.native),
                out,
            ))
        }
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let installed = Self::installed().await?;
        Ok(only_version(&installed, &candidate.id.native).map(str::to_string))
    }
}

/// The version of one app, or `None` when it is listed with different ones:
/// the same app can be installed for the system and for the user, and one
/// command updates both.
fn only_version<'a>(rows: &'a [(String, String)], app: &str) -> Option<&'a str> {
    let mut versions = rows
        .iter()
        .filter(|(a, _)| a == app)
        .map(|(_, v)| v.as_str());
    let first = versions.next()?;
    versions.all(|v| v == first).then_some(first)
}

fn join(offered: &[(String, String)], installed: &[(String, String)]) -> Vec<UpdateCandidate> {
    let mut found: Vec<UpdateCandidate> = Vec::new();
    for (app, _) in offered {
        if found.iter().any(|c| c.id.native == *app) {
            continue;
        }
        found.push(UpdateCandidate {
            id: PackageId::new(BackendKind::Flatpak, app.as_str()),
            name: app.clone(),
            installed: Version::parse(only_version(installed, app).unwrap_or("")),
            available: Version::parse(only_version(offered, app).unwrap_or("")),
            size_bytes: None,
            expected_sha256: None,
        });
    }
    found
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
            // Tab-separated by contract; the version column may be empty.
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
    use crate::lang_backends::assert_refuses_bad_input;

    #[test]
    fn parses_tab_separated_columns() {
        for out in [
            "org.mozilla.firefox\t141.0\norg.gimp.GIMP\t2.10.38\n",
            "org.mozilla.firefox\t141.0\r\norg.gimp.GIMP\t2.10.38\r\n",
        ] {
            let rows = parse_columns(out);
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0], ("org.mozilla.firefox".into(), "141.0".into()));
            assert_eq!(rows[1].1, "2.10.38");
        }
    }

    #[test]
    fn an_app_with_no_version_still_parses() {
        // Flatpak leaves the version blank for some apps; policy rejects it
        // later, but the scan must not lose the row or error.
        let rows = parse_columns("org.example.App\t\n");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "");
        assert!(!Version::parse(&rows[0].1).is_known());
    }

    #[test]
    fn blank_lines_are_skipped() {
        assert!(parse_columns("\n\n   \n").is_empty());
    }

    #[test]
    fn an_app_installed_twice_is_one_package() {
        let offered = parse_columns("org.a.A\t2.0\norg.a.A\t2.0\norg.b.B\t3.0\n");
        let installed = parse_columns("org.a.A\t1.0\norg.a.A\t1.5\norg.b.B\t2.0\n");
        let found = join(&offered, &installed);
        assert_eq!(found.len(), 2);
        // Two different versions: which one would be updated is not knowable.
        assert!(!found[0].installed.is_known());
        assert!(found[1].installed.is_upgrade_to(&found[1].available));
    }

    #[test]
    fn a_moved_target_is_refused() {
        let found = join(
            &parse_columns("org.b.B\t3.0\n"),
            &parse_columns("org.b.B\t2.0\n"),
        );
        let now = parse_columns("org.b.B\t3.1\n");
        assert!(guard::same_target(&found[0], only_version(&now, "org.b.B")).is_err());
        assert!(guard::same_target(&found[0], Some("3.0")).is_ok());
    }

    #[tokio::test]
    async fn apply_refuses_options_and_unknown_targets() {
        assert_refuses_bad_input(&FlatpakBackend::new(), "org.example.App", None).await;
    }
}
