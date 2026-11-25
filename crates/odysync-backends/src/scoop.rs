//! Scoop backend for Windows (user-scoped).
//!
//! Scoop is a user-level package manager for Windows that refuses to run
//! elevated. It installs packages into the user's home directory without
//! requiring administrator privileges.
//!
//! `scoop update <app>` takes no version, and before it starts it fetches new
//! manifests when the last time was more than three hours ago, which moves the
//! target. So `apply` fetches them itself, reads the offer again and refuses a
//! target that moved; the update that follows then has nothing left to fetch.
//!
//! Commands used:
//!   - `scoop status --local`: outdated apps
//!   - `scoop update`: fetch manifests
//!   - `scoop update <app>`: update one app
//!   - `scoop list`: installed apps
//!
//! Reference: <https://github.com/ScoopInstaller/Scoop/blob/master/libexec/scoop-status.ps1>

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::Result;
use odysync_core::model::{BackendKind, InstalledPackage, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

use crate::guard;

const SCAN_TIMEOUT: Duration = Duration::from_secs(120);
/// Fetching manifests is a git pull per bucket.
const SYNC_TIMEOUT: Duration = Duration::from_secs(300);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub struct ScoopBackend;

impl ScoopBackend {
    pub fn new() -> Self {
        Self
    }

    async fn outdated() -> Result<Vec<UpdateCandidate>> {
        // The versions come from the manifests on disk either way; without
        // `--local` scoop only adds a git fetch per bucket.
        let out = proc::run("scoop", &["status", "--local"], SCAN_TIMEOUT).await?;
        if !out.success() {
            return Err(guard::failed("scoop status", out));
        }
        Ok(parse_scoop_status(&out.stdout))
    }

    async fn installed() -> Result<Vec<ScoopApp>> {
        let out = proc::run("scoop", &["list"], SCAN_TIMEOUT).await?;
        if !out.success() {
            return Err(guard::failed("scoop list", out));
        }
        Ok(parse_scoop_list(&out.stdout))
    }
}

impl Default for ScoopBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for ScoopBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Scoop
    }

    fn display_name(&self) -> &str {
        "Scoop"
    }

    async fn is_available(&self) -> bool {
        cfg!(windows) && proc::exists("scoop", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let outdated = Self::outdated().await?;
        if outdated.is_empty() {
            return Ok(outdated);
        }
        Ok(without_global(outdated, &Self::installed().await?))
    }

    async fn list_installed(&self) -> Result<Vec<InstalledPackage>> {
        Ok(Self::installed()
            .await?
            .into_iter()
            .map(|app| InstalledPackage {
                id: PackageId::new(BackendKind::Scoop, app.name.as_str()),
                name: app.name,
                version: app.version,
            })
            .collect())
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        guard::precheck(candidate)?;

        // Scoop must not run elevated.
        if odysync_core::platform::is_elevated() {
            return Err(guard::refused(
                candidate,
                "Scoop must not be run as administrator",
            ));
        }
        let name = candidate.id.native.as_str();

        let sync = proc::run("scoop", &["update"], SYNC_TIMEOUT).await?;
        if !sync.success() {
            return Err(guard::failed("scoop update", sync));
        }
        let outdated = Self::outdated().await?;
        guard::same_target(
            candidate,
            outdated
                .iter()
                .find(|c| c.id == candidate.id)
                .map(|c| c.available.raw()),
        )?;
        guard::still_older(
            candidate,
            self.installed_version(candidate).await?.as_deref(),
        )?;

        let out = proc::run("scoop", &["update", name], INSTALL_TIMEOUT).await?;
        if out.success() {
            Ok(())
        } else {
            Err(guard::failed(format!("scoop update {name}"), out))
        }
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        // Without a query: scoop reads that as a regular expression.
        Ok(Self::installed()
            .await?
            .into_iter()
            .find(|app| !app.global && app.name.eq_ignore_ascii_case(&candidate.id.native))
            .map(|app| app.version))
    }
}

/// Split the table PowerShell's `Format-Table` writes.
///
/// The row of dashes under the header says where each column begins, which is
/// what keeps a cell with a space in it (`Held package`) or an empty one from
/// shifting the others.
fn parse_table(stdout: &str) -> Vec<Vec<String>> {
    let lines: Vec<Vec<char>> = stdout
        .lines()
        .map(|l| strip_ansi(l).trim_end().chars().collect())
        .collect();
    let Some(rule) = lines.iter().position(|l| {
        l.len() >= 4
            && l.starts_with(&['-', '-', '-', '-'])
            && l.iter().all(|c| matches!(c, '-' | ' '))
    }) else {
        return Vec::new();
    };
    let dashes = &lines[rule];
    let starts: Vec<usize> = (0..dashes.len())
        .filter(|&i| dashes[i] == '-' && (i == 0 || dashes[i - 1] == ' '))
        .collect();

    lines[rule + 1..]
        .iter()
        // A blank line ends the table.
        .take_while(|l| !l.is_empty())
        .map(|line| {
            starts
                .iter()
                .enumerate()
                .map(|(i, &start)| {
                    let end = starts.get(i + 1).copied().unwrap_or(usize::MAX);
                    let end = end.min(line.len());
                    line.get(start..end)
                        .map(|cell| cell.iter().collect::<String>().trim().to_string())
                        .unwrap_or_default()
                })
                .collect()
        })
        .collect()
}

/// PowerShell colours its output when it believes a person is reading.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else if c != '\r' {
            out.push(c);
        }
    }
    out
}

/// A cell that did not fit its column: what is left of it is not a version.
fn truncated(cell: &str) -> bool {
    cell.ends_with('\u{2026}') || cell.ends_with("...")
}

/// Parse `scoop status`.
///
/// ```text
/// Name Installed Version Latest Version Missing Dependencies Info
/// ---- ----------------- -------------- -------------------- ----
/// 7zip 24.07             24.09
/// ```
///
/// Apps that are not outdated are listed too, when something else is wrong
/// with them; those have no latest version. A held app is left alone, and so
/// is one whose last installation failed, which scoop refuses to update.
fn parse_scoop_status(stdout: &str) -> Vec<UpdateCandidate> {
    let mut found: Vec<UpdateCandidate> = Vec::new();
    for row in parse_table(stdout) {
        let [name, installed, latest, rest @ ..] = row.as_slice() else {
            continue;
        };
        let info = rest.get(1).map(String::as_str).unwrap_or_default();
        if name.is_empty()
            || latest.is_empty()
            || [name, installed, latest].iter().any(|c| truncated(c))
            || info.contains("Held package")
            || info.contains("Install failed")
            || info.contains("Manifest removed")
            || truncated(info)
        {
            continue;
        }
        // The same app installed for the user and for everyone.
        if let Some(i) = found.iter().position(|c| c.id.native == *name) {
            found.remove(i);
            continue;
        }
        found.push(UpdateCandidate {
            id: PackageId::new(BackendKind::Scoop, name.as_str()),
            name: name.clone(),
            installed: Version::parse(installed),
            available: Version::parse(latest),
            size_bytes: None,
            expected_sha256: None,
        });
    }
    found
}

/// One row of `scoop list`.
#[derive(Debug, PartialEq, Eq)]
struct ScoopApp {
    name: String,
    version: String,
    /// Installed for every user, which takes rights Scoop must not run with.
    global: bool,
}

/// Parse `scoop list`.
///
/// ```text
/// Installed apps:
///
/// Name Version Source Updated             Info
/// ---- ------- ------ -------             ----
/// 7zip 26.02   main   2026-08-23 00:06:14
/// ```
fn parse_scoop_list(stdout: &str) -> Vec<ScoopApp> {
    parse_table(stdout)
        .into_iter()
        .filter_map(|row| {
            let name = row.first()?.clone();
            let version = row.get(1)?.clone();
            if name.is_empty() || version.is_empty() || truncated(&name) || truncated(&version) {
                return None;
            }
            let global = row.last().is_some_and(|i| i.contains("Global install"));
            Some(ScoopApp {
                name,
                version,
                global,
            })
        })
        .collect()
}

fn without_global(found: Vec<UpdateCandidate>, installed: &[ScoopApp]) -> Vec<UpdateCandidate> {
    found
        .into_iter()
        .filter(|c| {
            !installed
                .iter()
                .any(|app| app.global && app.name.eq_ignore_ascii_case(&c.id.native))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang_backends::assert_refuses_bad_input;

    fn crlf(text: &str) -> String {
        text.replace('\n', "\r\n")
    }

    /// Columns and texts from scoop-status.ps1 and its format file
    /// (supporting/formats/ScoopTypes.Format.ps1xml), laid out like the
    /// captured `scoop list` below.
    const STATUS: &str = "\
WARN  Scoop bucket(s) out of date. Run 'scoop update' to get the latest changes.

Name    Installed Version Latest Version Missing Dependencies Info
----    ----------------- -------------- -------------------- ----
7zip    24.07             24.09
broken  1.0.0             1.1.0                               Install failed
git     2.45.0            2.46.0         7zip | innounp
gone    3.0                                                   Manifest removed
nodejs  20.0.0            22.0.0                              Held package
old     1.0               2.0                                 Deprecated


";

    #[test]
    fn parses_the_status_table() {
        for text in [STATUS.to_string(), crlf(STATUS)] {
            let found = parse_scoop_status(&text);
            let ids: Vec<&str> = found.iter().map(|c| c.id.native.as_str()).collect();
            assert_eq!(ids, ["7zip", "git", "old"]);
            assert_eq!(found[0].installed.raw(), "24.07");
            assert_eq!(found[0].available.raw(), "24.09");
            // The missing dependencies did not shift anything.
            assert_eq!(found[1].available.raw(), "2.46.0");
        }
    }

    #[test]
    fn a_coloured_table_parses_the_same() {
        let coloured = STATUS
            .replace("Name    Inst", "\u{1b}[32;1mName    Inst")
            .replace("----    ---", "\u{1b}[32;1m----    ---")
            .replace("Info\n", "Info\u{1b}[0m\n");
        assert_eq!(parse_scoop_status(&coloured).len(), 3);
    }

    #[test]
    fn empty_output_yields_empty_vec() {
        assert!(parse_scoop_status("").is_empty());
        // Captured: what scoop says when there is nothing to do.
        assert!(parse_scoop_status("Everything is ok!\r\n").is_empty());
    }

    #[test]
    fn cut_off_cells_are_not_versions() {
        let text = "\
Name Installed Version Latest Version Missing Dependencies Info
---- ----------------- -------------- -------------------- ----
a    1.0.0-20240101... 1.0.0-2024...
b    1.0               2.0\u{2026}
c    1.0               2.0
";
        let found = parse_scoop_status(text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id.native, "c");
    }

    #[test]
    fn an_app_that_is_listed_twice_is_left_alone() {
        let text = "\
Name Installed Version Latest Version Missing Dependencies Info
---- ----------------- -------------- -------------------- ----
git  2.45.0            2.46.0
git  2.44.0            2.46.0
jq   1.6               1.7
";
        let found = parse_scoop_status(text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id.native, "jq");
    }

    /// Captured from `scoop list` on Windows, with the line endings it wrote.
    const LIST: &str = "Installed apps:\n\r\nName Version Source Updated             Info\r\n---- ------- ------ -------             ----\r\n7zip 26.02   main   2026-08-23 00:06:14     \r\nqemu 11.1.0  main   2026-08-23 00:07:37     \r\n\r\n\r\n";

    #[test]
    fn parses_scoop_list() {
        for text in [LIST.to_string(), LIST.replace('\r', "")] {
            let installed = parse_scoop_list(&text);
            assert_eq!(installed.len(), 2);
            assert_eq!(installed[0].name, "7zip");
            assert_eq!(installed[0].version, "26.02");
            assert_eq!(installed[1].name, "qemu");
            assert_eq!(installed[1].version, "11.1.0");
            assert!(!installed.iter().any(|a| a.global));
        }
    }

    #[test]
    fn scoop_list_ignores_headers_and_blank_lines() {
        assert!(parse_scoop_list("Installed apps:\n\nName  Version\n----  -------\n").is_empty());
        assert!(parse_scoop_list("").is_empty());
    }

    #[test]
    fn apps_installed_for_everyone_are_left_alone() {
        let output = "\
Name    Version   Source Updated             Info
----    -------   ------ -------             ----
git     2.45.0    main   2024-09-01 10:00:00 Global install
jq      1.6       main   2024-09-01 10:00:00 Held package
plain   2.0.0
";
        let installed = parse_scoop_list(output);
        assert_eq!(installed.len(), 3);
        assert!(installed[0].global);
        assert!(!installed[1].global);
        assert_eq!(installed[2].version, "2.0.0");

        let found = parse_scoop_status(STATUS);
        let left = without_global(found, &installed);
        assert!(!left.iter().any(|c| c.id.native == "git"));
        assert_eq!(left.len(), 2);
    }

    #[test]
    fn a_moved_target_is_refused() {
        let found = parse_scoop_status(STATUS);
        assert!(guard::same_target(&found[0], Some("24.09")).is_ok());
        assert!(guard::same_target(&found[0], Some("24.10")).is_err());
        // `scoop update *` updates everything; it is not an installed app.
        assert!(guard::still_older(&found[0], None).is_err());
    }

    #[test]
    fn backend_kind_is_correct() {
        let b = ScoopBackend::new();
        assert_eq!(b.kind(), BackendKind::Scoop);
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_options_and_unknown_targets() {
        assert_refuses_bad_input(&ScoopBackend::new(), "firefox", None).await;
    }
}
