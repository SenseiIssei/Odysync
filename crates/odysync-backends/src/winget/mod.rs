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
        if self.store {
            "msstore"
        } else {
            "winget"
        }
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

        let kind = self.kind();
        Ok(rows
            .into_iter()
            // winget lists Store packages inside the winget table and vice
            // versa; keep each backend to its own source so elevation rules
            // are applied correctly.
            .filter(|r| r.source.is_empty() || r.source.eq_ignore_ascii_case(self.source()))
            .map(|r| UpdateCandidate {
                id: PackageId::new(kind, r.id),
                name: r.name,
                installed: Version::parse(&r.version),
                available: Version::parse(&r.available),
                size_bytes: None,
                // winget verifies the installer hash against its own manifest
                // internally; we do not have the digest to re-check here.
                expected_sha256: None,
            })
            .collect())
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
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }

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

        if out.success() {
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
        let rows = table::parse(&out.stdout);

        Ok(rows
            .into_iter()
            .find(|r| r.id.eq_ignore_ascii_case(&candidate.id.native))
            .map(|r| r.version))
    }
}

/// Turn winget's numeric exit codes into something a user can act on.
///
/// These are the documented APPINSTALLER_CLI_ERROR values; surfacing them by
/// name is the difference between "it failed" and knowing a reboot is pending.
fn describe_exit(code: i32, stderr: &str, stdout: &str) -> String {
    // winget reports these as unsigned hex in its docs.
    let hint = match code as u32 {
        0x8A15002B => Some("no applicable installer for this system"),
        0x8A150109 => Some("a reboot is required before this package can be updated"),
        0x8A150049 => Some("the package is pinned in winget itself"),
        0x8A15010D => Some("the installer hash did not match winget's manifest"),
        0x8A150011 => Some("the installer failed its own integrity check"),
        0x8A150056 => Some("this package must be updated from the Microsoft Store app"),
        0x8A150044 => Some("administrator rights are required for this package"),
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
        let msg = describe_exit(0x8A150109u32 as i32, "", "");
        assert!(msg.contains("reboot is required"), "got: {msg}");

        let msg = describe_exit(0x8A15010Du32 as i32, "", "");
        assert!(msg.contains("hash did not match"), "got: {msg}");
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
