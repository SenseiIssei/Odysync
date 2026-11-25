//! The winget backend.
//!
//! winget stays as the transport — nothing else covers as much Windows software
//! — but every call is wrapped so the failure modes that corrupted installs
//! cannot recur:
//!
//!   * **Exact version pinning.** `--version <target>` is always passed, so a
//!     scan and the install that follows can never disagree about what is being
//!     installed, even if the source publishes a newer manifest in between.
//!   * **No reinstall fallback.** A failed upgrade stays failed. The old code
//!     ran `winget install` on failure, which reinstalls over a working copy and
//!     is what forced manual repair.
//!   * **No `--include-unknown`.** Packages whose installed version winget
//!     cannot read are still *listed* (policy explains why they were skipped)
//!     but never acted on.
//!   * **Silent and windowless.** `--disable-interactivity` plus
//!     `CREATE_NO_WINDOW` means no console flashes and no install can block
//!     forever on a hidden prompt.

pub mod table;

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, InstalledPackage, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

use crate::guard;

/// Scans can be slow on a cold source index, but must not hang a service.
const SCAN_TIMEOUT: Duration = Duration::from_secs(180);
/// Large installers (Office, toolchains) legitimately take a long time.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(45 * 60);
const QUERY_TIMEOUT: Duration = Duration::from_secs(60);

pub struct WingetBackend {
    /// When true, target Microsoft Store packages instead of the winget source.
    store: bool,
}

impl WingetBackend {
    /// The winget community source.
    pub fn new() -> Self {
        Self { store: false }
    }

    /// The Microsoft Store source, which must run unelevated.
    pub fn store() -> Self {
        Self { store: true }
    }

    fn source(&self) -> &'static str {
        if self.store { "msstore" } else { "winget" }
    }

    /// Flags every winget invocation carries.
    ///
    /// `--disable-interactivity` is the one that makes background operation
    /// possible: without it winget can stop on a prompt that nobody can see.
    fn common_args(&self) -> Vec<String> {
        vec![
            "--accept-source-agreements".into(),
            "--disable-interactivity".into(),
            "--source".into(),
            self.source().into(),
        ]
    }
}

impl Default for WingetBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for WingetBackend {
    fn kind(&self) -> BackendKind {
        if self.store {
            BackendKind::MsStore
        } else {
            BackendKind::Winget
        }
    }

    fn display_name(&self) -> &str {
        if self.store {
            "Microsoft Store"
        } else {
            "Windows Package Manager (winget)"
        }
    }

    async fn is_available(&self) -> bool {
        cfg!(windows) && proc::exists("winget", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let mut args = vec!["upgrade".to_string()];
        args.extend(self.common_args());

        let out = proc::run("winget", &args, SCAN_TIMEOUT).await?;

        // winget exits non-zero when there is simply nothing to upgrade, so a
        // failed exit code with a parseable (or empty) table is not an error.
        let rows = table::parse(&out.stdout);
        if rows.is_empty() && !out.success() && out.stdout.trim().is_empty() {
            return Err(Error::CommandFailed {
                command: "winget upgrade".into(),
                code: out.code,
                stderr: out.stderr,
            });
        }

        let rows: Vec<table::Row> = rows
            .into_iter()
            // winget lists Store packages inside the winget table and vice
            // versa; keep each backend to its own source so elevation rules
            // are applied correctly.
            .filter(|r| r.source.is_empty() || r.source.eq_ignore_ascii_case(self.source()))
            .collect();
        Ok(candidates(self.kind(), &rows))
    }

    async fn list_installed(&self) -> Result<Vec<InstalledPackage>> {
        let mut args = vec!["list".to_string()];
        args.extend(self.common_args());

        let out = proc::run("winget", &args, SCAN_TIMEOUT).await?;

        // Same tolerance as `scan`: winget can exit non-zero while still having
        // printed a usable table, so only a non-zero exit with no output at all
        // is a real failure.
        let rows = table::parse_list(&out.stdout);
        if rows.is_empty() && !out.success() && out.stdout.trim().is_empty() {
            return Err(Error::CommandFailed {
                command: "winget list".into(),
                code: out.code,
                stderr: out.stderr,
            });
        }

        let kind = self.kind();
        Ok(rows
            .into_iter()
            // winget lists Store packages inside the winget table and vice
            // versa; keep each backend to its own source, exactly as `scan`
            // does, so the two views agree about which package belongs where.
            .filter(|r| r.source.is_empty() || r.source.eq_ignore_ascii_case(self.source()))
            .map(|r| InstalledPackage {
                id: PackageId::new(kind, r.id),
                name: r.name,
                version: r.version,
            })
            .collect())
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        // Refuse to run without a concrete target version. Without `--version`
        // winget resolves "latest" itself, which reintroduces the gap between
        // what policy approved and what actually gets installed.
        guard::precheck(candidate)?;

        let mut args = vec![
            "upgrade".to_string(),
            "--id".to_string(),
            candidate.id.native.clone(),
            // Match the id exactly; a substring match can select a different
            // package entirely.
            "--exact".to_string(),
            "--version".to_string(),
            candidate.available.raw().to_string(),
            "--silent".to_string(),
            "--accept-package-agreements".to_string(),
        ];
        args.extend(self.common_args());

        let out = proc::run("winget", &args, INSTALL_TIMEOUT).await?;

        if out.success() || finishes_with_restart(out.code) {
            return Ok(());
        }

        // Deliberately terminal: no interactive retry, no `winget install`
        // fallback. The caller reports the failure and the package is left
        // exactly as it was.
        Err(Error::CommandFailed {
            command: format!("winget upgrade --id {} --exact", candidate.id.native),
            code: out.code,
            stderr: describe_exit(out.code, &out.stderr, &out.stdout),
        })
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let mut args = vec![
            "list".to_string(),
            "--id".to_string(),
            candidate.id.native.clone(),
            "--exact".to_string(),
        ];
        args.extend(self.common_args());

        let out = proc::run("winget", &args, QUERY_TIMEOUT).await?;
        let rows = table::parse_list(&out.stdout);

        // The same program can be listed more than once, for example for this
        // user and for everyone, or with an older version next to it.
        let versions: Vec<&str> = rows
            .iter()
            .filter(|r| r.id.eq_ignore_ascii_case(&candidate.id.native))
            .map(|r| r.version.as_str())
            .collect();
        Ok(newest_installed(&versions).map(str::to_string))
    }
}

/// The newest of the installed versions, when every one of them can be read.
fn newest_installed<'a>(versions: &[&'a str]) -> Option<&'a str> {
    if versions.iter().any(|v| table::is_truncated(v)) {
        return None;
    }
    guard::newest(versions.iter().copied(), Version::parse)
}

/// One candidate per package id.
///
/// winget lists a program once per entry in the list of installed programs, so
/// an id can appear several times. The newest entry decides: an older one next
/// to it is not a reason to update. Where the entries cannot be ordered the
/// installed version is unknown, and policy refuses.
fn candidates(kind: BackendKind, rows: &[table::Row]) -> Vec<UpdateCandidate> {
    let mut found: Vec<UpdateCandidate> = Vec::new();
    for row in rows {
        let same_id = |id: &str| id.eq_ignore_ascii_case(&row.id);
        if found.iter().any(|c| same_id(&c.id.native)) {
            continue;
        }
        let same: Vec<&table::Row> = rows.iter().filter(|r| same_id(&r.id)).collect();
        let installed: Vec<&str> = same.iter().map(|r| r.version.as_str()).collect();
        let one_target = same.iter().all(|r| r.available == row.available);
        let available = if one_target && !table::is_truncated(&row.available) {
            Version::parse(&row.available)
        } else {
            Version::Unknown(row.available.clone())
        };
        found.push(UpdateCandidate {
            id: PackageId::new(kind, row.id.as_str()),
            name: row.name.clone(),
            installed: Version::parse(newest_installed(&installed).unwrap_or_default()),
            available,
            size_bytes: None,
            // winget verifies the installer hash against its own manifest
            // internally; we do not have the digest to re-check here.
            expected_sha256: None,
        });
    }
    found
}

/// The update is installed and the machine has to restart to finish it:
/// INSTALL_REBOOT_REQUIRED_TO_FINISH and INSTALL_REBOOT_INITIATED.
fn finishes_with_restart(code: i32) -> bool {
    matches!(code as u32, 0x8A150109 | 0x8A15010B)
}

/// Turn winget's numeric exit codes into something a user can act on.
///
/// These are the documented APPINSTALLER_CLI_ERROR values; surfacing them by
/// name is the difference between "it failed" and knowing a reboot is pending.
fn describe_exit(code: i32, stderr: &str, stdout: &str) -> String {
    // winget reports these as unsigned hex in its docs:
    // https://github.com/microsoft/winget-cli/blob/master/doc/windows/package-manager/winget/returnCodes.md
    let hint = match code as u32 {
        0x8A150011 => Some("the installer hash did not match winget's manifest"),
        0x8A150014 => Some("winget no longer finds this package"),
        0x8A150019 => Some("administrator rights are required for this package"),
        0x8A15002B => Some("winget has no applicable update for this package"),
        0x8A150049 => Some("running the MSI installer failed"),
        0x8A15004F => Some("the approved version is not newer than the installed one"),
        0x8A150056 => Some("this installer must not be run as administrator"),
        0x8A150068 => Some("the package is pinned in winget itself"),
        0x8A15010A => Some("a restart is required before this package can be updated"),
        0x8A15010C => Some("the installation was cancelled"),
        0x8A15010D => Some("another version of this application is already installed"),
        0x8A15010E => Some("a higher version of this application is already installed"),
        _ => None,
    };

    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };

    match hint {
        Some(h) => format!("{h} (winget 0x{:08X}). {detail}", code as u32),
        None => format!("winget exit 0x{:08X}. {detail}", code as u32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_winget_error_codes_get_a_readable_hint() {
        let msg = describe_exit(0x8A15010Au32 as i32, "", "");
        assert!(msg.contains("restart is required"), "got: {msg}");

        let msg = describe_exit(0x8A15010Du32 as i32, "", "");
        assert!(msg.contains("already installed"), "got: {msg}");

        let msg = describe_exit(0x8A150011u32 as i32, "", "");
        assert!(msg.contains("hash did not match"), "got: {msg}");
    }

    #[test]
    fn an_update_that_needs_a_restart_to_finish_is_a_success() {
        assert!(finishes_with_restart(0x8A150109u32 as i32));
        assert!(finishes_with_restart(0x8A15010Bu32 as i32));
        // These did not install anything.
        for code in [0x8A15010Au32, 0x8A15010D, 0x8A15010E, 0x8A15002B, 1] {
            assert!(!finishes_with_restart(code as i32), "{code:#x}");
        }
    }

    fn row(id: &str, version: &str, available: &str) -> table::Row {
        table::Row {
            name: id.into(),
            id: id.into(),
            version: version.into(),
            available: available.into(),
            source: "winget".into(),
        }
    }

    #[test]
    fn a_package_that_is_listed_twice_is_one_candidate() {
        let found = candidates(
            BackendKind::Winget,
            &[
                row("A.A", "1.9", "2.0"),
                row("B.B", "1.0", "1.1"),
                row("a.a", "1.10", "2.0"),
                row("C.C", "1.0", "3.0"),
                row("C.C", "Unknown", "3.0"),
                row("D.D", "1.0", "2.0"),
                row("D.D", "1.0", "2.1"),
                row("E.E", "1.0", "2.0.1\u{2026}"),
                row("F.F", "1.0.1\u{2026}", "2.0"),
            ],
        );
        assert_eq!(found.len(), 6);
        assert_eq!(found[0].installed.raw(), "1.10");
        assert!(found[1].installed.is_upgrade_to(&found[1].available));
        // Not comparable, two different targets, cut off: never acted on.
        assert!(!found[2].installed.is_known());
        assert!(!found[3].available.is_known());
        assert!(!found[4].available.is_known());
        assert!(!found[5].installed.is_known());
    }

    #[test]
    fn the_newest_of_several_installed_versions_is_read_back() {
        assert_eq!(newest_installed(&["1.9", "1.10"]), Some("1.10"));
        assert_eq!(newest_installed(&["1.9", "Unknown"]), None);
        assert_eq!(newest_installed(&["1.9\u{2026}"]), None);
        assert_eq!(newest_installed(&[]), None);
    }

    #[tokio::test]
    async fn apply_refuses_options_as_ids() {
        for backend in [WingetBackend::new(), WingetBackend::store()] {
            crate::lang_backends::assert_refuses_bad_input(&backend, "Vendor.App", None).await;
        }
    }

    #[test]
    fn unknown_codes_still_surface_the_raw_output() {
        let msg = describe_exit(1, "something broke", "");
        assert!(msg.contains("something broke"), "got: {msg}");
        assert!(msg.contains("0x00000001"), "got: {msg}");
    }

    #[test]
    fn stdout_is_used_when_stderr_is_empty() {
        let msg = describe_exit(1, "   ", "detail on stdout");
        assert!(msg.contains("detail on stdout"), "got: {msg}");
    }

    #[tokio::test]
    async fn apply_refuses_a_candidate_without_a_known_target_version() {
        // Guards the pinning invariant: no exact version, no install.
        let backend = WingetBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::Winget, "Vendor.App"),
            name: "App".into(),
            installed: Version::parse("1.0.0"),
            available: Version::parse("Unknown"),
            size_bytes: None,
            expected_sha256: None,
        };
        let err = backend.apply(&candidate).await.unwrap_err();
        assert!(matches!(err, Error::Verification { .. }));
    }

    #[test]
    fn every_invocation_disables_interactivity_and_pins_the_source() {
        let args = WingetBackend::new().common_args();
        assert!(args.iter().any(|a| a == "--disable-interactivity"));
        assert!(args.iter().any(|a| a == "winget"));

        let store = WingetBackend::store().common_args();
        assert!(store.iter().any(|a| a == "msstore"));
    }

    #[test]
    fn store_and_winget_map_to_distinct_backend_kinds() {
        assert_eq!(WingetBackend::new().kind(), BackendKind::Winget);
        assert_eq!(WingetBackend::store().kind(), BackendKind::MsStore);
    }
}
