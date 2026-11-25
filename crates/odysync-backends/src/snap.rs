//! Snap backend for Linux.
//!
//! Snap packages are containerised applications with automatic update support.
//! Snap verifies package signatures via its store infrastructure.
//!
//! The version of a snap is free text that may stay the same from one build to
//! the next, and no command accepts it. What identifies a build, only ever
//! grows, and can be asked for, is the revision, so revisions are what this
//! backend compares and pins.
//!
//! Commands used:
//!   - `snap refresh --list`: pending updates with their revisions
//!   - `snap list`: installed revisions and held snaps
//!   - `snap refresh <name> --revision=<rev>`: update to exactly that build

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::Result;
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

use crate::guard;

const SCAN_TIMEOUT: Duration = Duration::from_secs(60);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);
const QUERY_TIMEOUT: Duration = Duration::from_secs(30);

pub struct SnapBackend;

impl SnapBackend {
    pub fn new() -> Self {
        Self
    }

    async fn pending() -> Result<Vec<SnapRow>> {
        let out = proc::run("snap", &["refresh", "--list"], SCAN_TIMEOUT).await?;
        if !out.success() {
            return Err(guard::failed("snap refresh --list", out));
        }
        Ok(parse_table(&out.stdout))
    }

    async fn installed() -> Result<Vec<SnapRow>> {
        let out = proc::run("snap", &["list"], QUERY_TIMEOUT).await?;
        if !out.success() {
            return Err(guard::failed("snap list", out));
        }
        Ok(parse_table(&out.stdout))
    }
}

impl Default for SnapBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for SnapBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Snap
    }

    fn display_name(&self) -> &str {
        "Snap"
    }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && proc::exists("snap", &["version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let pending = Self::pending().await?;
        if pending.is_empty() {
            return Ok(Vec::new());
        }
        Ok(join(pending, &Self::installed().await?))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        guard::precheck(candidate)?;
        let name = candidate.id.native.as_str();

        let installed = Self::installed().await?;
        let current = installed.iter().find(|r| r.name == name);
        // An explicit refresh goes through a hold, so the hold has to be
        // respected here.
        if current.is_some_and(SnapRow::held) {
            return Err(guard::refused(candidate, "the snap is held"));
        }
        guard::still_older(candidate, current.map(|r| r.revision.as_str()))?;
        let pending = Self::pending().await?;
        guard::same_target(
            candidate,
            pending
                .iter()
                .find(|r| r.name == name)
                .map(|r| r.revision.as_str()),
        )?;

        let args = refresh_args(name, candidate.available.raw());
        let out = proc::run("snap", &args, INSTALL_TIMEOUT).await?;
        if out.success() {
            Ok(())
        } else {
            Err(guard::failed(format!("snap refresh {name}"), out))
        }
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        Ok(Self::installed()
            .await?
            .into_iter()
            .find(|r| r.name == candidate.id.native)
            .map(|r| r.revision))
    }
}

fn refresh_args(name: &str, revision: &str) -> Vec<String> {
    vec![
        "refresh".to_string(),
        name.to_string(),
        format!("--revision={revision}"),
    ]
}

/// One row of `snap list` or `snap refresh --list`. Both begin with name,
/// version and revision, and both end with notes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SnapRow {
    name: String,
    version: String,
    revision: String,
    notes: String,
}

impl SnapRow {
    /// Notes are separated by commas: `classic,held`.
    fn held(&self) -> bool {
        self.notes.split(',').any(|n| n == "held")
    }
}

/// Parse either table. No cell contains a space, and an empty one is `-`.
fn parse_table(stdout: &str) -> Vec<SnapRow> {
    stdout
        .lines()
        .skip_while(|l| !l.trim_start().starts_with("Name "))
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 4 {
                return None;
            }
            Some(SnapRow {
                name: fields[0].to_string(),
                version: fields[1].to_string(),
                revision: fields[2].to_string(),
                notes: fields[fields.len() - 1].to_string(),
            })
        })
        .collect()
}

fn join(pending: Vec<SnapRow>, installed: &[SnapRow]) -> Vec<UpdateCandidate> {
    pending
        .into_iter()
        .filter_map(|new| {
            let old = installed.iter().find(|r| r.name == new.name)?;
            if old.held() {
                return None;
            }
            Some(UpdateCandidate {
                id: PackageId::new(BackendKind::Snap, new.name.as_str()),
                name: format!("{} {}", new.name, new.version),
                // A snap that was installed from a file has a revision such as
                // `x1`, which does not parse: those are never updated.
                installed: Version::parse(&old.revision),
                available: Version::parse(&new.revision),
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

    /// Columns from `listRefresh` in snapd:
    /// https://github.com/canonical/snapd/blob/2.66/cmd/snap/cmd_snap_op.go
    const PENDING: &str = "\
Name      Version        Rev    Size   Publisher   Notes
firefox   141.0-1        4173   271MB  mozilla**   -
chromium  120.0.6099.71  2663   167MB  canonical** -
code      fa4b2d9f       150    330MB  vscode**    classic
local     1.0            12     1MB    me          -
";

    /// Columns from `cmd_list.go`, notes from `notes.go` of the same release.
    const INSTALLED: &str = "\
Name      Version        Rev    Tracking         Publisher   Notes
chromium  119.0.6045.1   2650   latest/stable    canonical** held
code      fa4b2d9f       149    latest/stable    vscode**    classic
core22    20240111       1122   latest/stable    canonical** base
firefox   140.0-2        4100   latest/stable/\u{2026} mozilla**   -
local     0.9            x1     -                -           devmode
";

    fn crlf(text: &str) -> String {
        text.replace('\n', "\r\n")
    }

    #[test]
    fn revisions_are_compared_and_held_snaps_left_alone() {
        for (pending, installed) in [
            (PENDING.to_string(), INSTALLED.to_string()),
            (crlf(PENDING), crlf(INSTALLED)),
        ] {
            let pending = parse_table(&pending);
            let installed = parse_table(&installed);
            assert_eq!(pending.len(), 4);
            assert_eq!(installed.len(), 5);

            let found = join(pending, &installed);
            let ids: Vec<&str> = found.iter().map(|c| c.id.native.as_str()).collect();
            assert_eq!(ids, ["firefox", "code", "local"]);
            assert_eq!(found[0].installed.raw(), "4100");
            assert_eq!(found[0].available.raw(), "4173");
            assert_eq!(found[0].name, "firefox 141.0-1");
            assert!(found[0].installed.is_upgrade_to(&found[0].available));
            // The version text did not change, the build did.
            assert!(found[1].installed.is_upgrade_to(&found[1].available));
            // Installed from a file: never an update.
            assert!(!found[2].installed.is_upgrade_to(&found[2].available));
        }
    }

    #[test]
    fn held_is_one_of_several_notes() {
        let mut row = parse_table(INSTALLED).remove(0);
        assert!(row.held());
        row.notes = "classic,held".into();
        assert!(row.held());
        row.notes = "base".into();
        assert!(!row.held());
    }

    #[test]
    fn empty_output_yields_empty_vec() {
        assert!(parse_table("").is_empty());
        // Printed on stderr by snap, but not a row in any case.
        assert!(parse_table("All snaps up to date.\n").is_empty());
    }

    #[test]
    fn header_only_yields_empty_vec() {
        assert!(parse_table("Name      Version  Rev  Size  Publisher  Notes\n").is_empty());
    }

    #[test]
    fn the_refresh_names_the_revision() {
        assert_eq!(
            refresh_args("firefox", "4173"),
            ["refresh", "firefox", "--revision=4173"]
        );
    }

    #[test]
    fn a_moved_revision_is_refused() {
        let found = join(parse_table(PENDING), &parse_table(INSTALLED));
        assert!(guard::same_target(&found[0], Some("4173")).is_ok());
        assert!(guard::same_target(&found[0], Some("4180")).is_err());
        assert!(guard::still_older(&found[0], Some("4173")).is_err());
        assert!(guard::still_older(&found[0], Some("x2")).is_err());
    }

    #[test]
    fn backend_kind_is_correct() {
        let b = SnapBackend::new();
        assert_eq!(b.kind(), BackendKind::Snap);
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_options_and_unknown_targets() {
        assert_refuses_bad_input(&SnapBackend::new(), "firefox", None).await;
    }
}
