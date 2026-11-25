//! Pacman backend for Arch Linux and derivatives. It reports, but does not
//! install.
//!
//! Arch does not support updating one package: every package is built against
//! the newest versions of all others, so a partial upgrade can break the
//! system (<https://wiki.archlinux.org/title/System_maintenance#Partial_upgrades_are_unsupported>).
//! It would not work anyway: `checkupdates` looks into a database of its own,
//! and `pacman -S pkg=new` looks into the one from the last full upgrade, which
//! does not know that version. Refreshing that one with `-Sy` is what creates
//! the partial upgrade. So the user runs the full `pacman -Syu`.
//!
//! Commands used:
//!   - `checkupdates`: list available updates (uses a temporary sync database)
//!   - `pacman -Q pkg`: read the installed version

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;
use odysync_core::version_native::Scheme;

use crate::guard;

const SCAN_TIMEOUT: Duration = Duration::from_secs(300);
const QUERY_TIMEOUT: Duration = Duration::from_secs(30);

pub struct PacmanBackend;

impl PacmanBackend {
    pub fn new() -> Self {
        Self
    }

    /// Fallback scan using `pacman -Qu`, which knows only what the last full
    /// upgrade saw.
    async fn scan_via_pacman() -> Result<Vec<UpdateCandidate>> {
        let out = proc::run("pacman", &["-Qu"], SCAN_TIMEOUT).await?;
        // Exit 1 without a message means that nothing is out of date.
        if !out.success() && !out.stderr.trim().is_empty() {
            return Err(guard::failed("pacman -Qu", out));
        }
        Ok(parse_updates(&out.stdout))
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
        match proc::run("checkupdates", &[] as &[&str], SCAN_TIMEOUT).await {
            // checkupdates exits 0 with updates on stdout, or 2 when there are
            // none.
            Ok(out) if out.code == 0 || out.code == 2 => Ok(parse_updates(&out.stdout)),
            Ok(out) => Err(guard::failed("checkupdates", out)),
            Err(e) => {
                // It comes with pacman-contrib, which may not be installed.
                tracing::warn!(error = %e, "checkupdates not available, falling back to pacman");
                Self::scan_via_pacman().await
            }
        }
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        guard::precheck(candidate)?;
        Err(Error::unavailable(
            "Pacman",
            format!(
                "Arch Linux does not support updating a single package ({}); run a full \
                 upgrade with `sudo pacman -Syu`",
                candidate.id.native
            ),
        ))
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let out = proc::run("pacman", &["-Q", "--", &candidate.id.native], QUERY_TIMEOUT).await?;
        if !out.success() {
            return Ok(None);
        }

        // Output: "name version"
        let line = out.stdout.lines().next().unwrap_or("").trim();
        Ok(line.split_whitespace().nth(1).map(str::to_string))
    }
}

/// Parse `checkupdates` and `pacman -Qu`: `name old_version -> new_version`.
///
/// `pacman -Qu` marks a package from `IgnorePkg` with `[ignored]`; those are
/// the user's holds.
fn parse_updates(stdout: &str) -> Vec<UpdateCandidate> {
    stdout
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let [name, old, "->", new] = fields.as_slice() else {
                return None;
            };
            Some(UpdateCandidate {
                id: PackageId::new(BackendKind::Pacman, *name),
                name: name.to_string(),
                installed: Version::parse_as(old, Scheme::Pacman),
                available: Version::parse_as(new, Scheme::Pacman),
                size_bytes: None,
                expected_sha256: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang_backends::assert_refuses_bad_input;

    #[test]
    fn parses_checkupdates_output() {
        // Format from https://man.archlinux.org/man/checkupdates.8 and
        // https://man.archlinux.org/man/pacman.8 (-Qu).
        let output = "\
firefox 140.0-1 -> 141.0-1
linux 6.8.9.arch1-1 -> 6.8.10.arch1-1
ca-certificates-utils 20240203-1 -> 1:20240203-1
";
        for text in [output.to_string(), output.replace('\n', "\r\n")] {
            let candidates = parse_updates(&text);
            assert_eq!(candidates.len(), 3);
            assert_eq!(candidates[0].id.native, "firefox");
            assert_eq!(candidates[0].installed.raw(), "140.0-1");
            assert_eq!(candidates[0].available.raw(), "141.0-1");
            assert_eq!(candidates[1].id.native, "linux");
            // Including the one where only the epoch moves.
            assert!(
                candidates
                    .iter()
                    .all(|c| c.installed.is_upgrade_to(&c.available))
            );
        }
    }

    #[test]
    fn empty_output_yields_empty_vec() {
        assert!(parse_updates("").is_empty());
    }

    #[test]
    fn skips_malformed_and_ignored_lines() {
        let output = "firefox 140.0-1 -> 141.0-1\nbad line no arrow\n\
                      linux 6.8-1 -> 6.9-1 [ignored]\n";
        let candidates = parse_updates(output);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id.native, "firefox");
    }

    #[test]
    fn backend_kind_is_correct() {
        let b = PacmanBackend::new();
        assert_eq!(b.kind(), BackendKind::Pacman);
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_sends_the_user_to_a_full_upgrade() {
        let candidates = parse_updates("firefox 140.0-1 -> 141.0-1\n");
        let err = PacmanBackend::new()
            .apply(&candidates[0])
            .await
            .unwrap_err();
        assert!(matches!(err, Error::BackendUnavailable { .. }));
        assert!(err.to_string().contains("pacman -Syu"));
    }

    #[tokio::test]
    async fn apply_refuses_options_and_unknown_targets() {
        assert_refuses_bad_input(&PacmanBackend::new(), "firefox", Some(Scheme::Pacman)).await;
    }
}
