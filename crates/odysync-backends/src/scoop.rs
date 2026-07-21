//! Scoop backend for Windows (user-scoped).
//!
//! Scoop is a user-level package manager for Windows that refuses to run
//! elevated. It installs packages into the user's home directory without
//! requiring administrator privileges.
//!
//! Commands used:
//!   - `scoop status` — list outdated packages (JSON output)
//!   - `scoop update <pkg>` — update a specific package
//!   - `scoop list <pkg>` — read installed version
//!
//! Reference: https://scoop.sh/

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, InstalledPackage, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

const SCAN_TIMEOUT: Duration = Duration::from_secs(60);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(300);
const QUERY_TIMEOUT: Duration = Duration::from_secs(30);

pub struct ScoopBackend;

impl ScoopBackend {
    pub fn new() -> Self {
        Self
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
        // `scoop status` outputs JSON when called with --json (if available),
        // otherwise text. We parse the text output as it's more stable.
        let out = proc::run("scoop", &["status"], SCAN_TIMEOUT).await?;

        if !out.success() {
            return Err(Error::CommandFailed {
                command: "scoop status".into(),
                code: out.code,
                stderr: out.stderr,
            });
        }

        Ok(parse_scoop_status(&out.stdout))
    }

    async fn list_installed(&self) -> Result<Vec<InstalledPackage>> {
        let out = proc::run("scoop", &["list"], SCAN_TIMEOUT).await?;

        if !out.success() {
            return Err(Error::CommandFailed {
                command: "scoop list".into(),
                code: out.code,
                stderr: out.stderr,
            });
        }

        Ok(parse_scoop_list(&out.stdout))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }

        // Scoop must not run elevated.
        if odysync_core::platform::is_elevated() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "Scoop must not be run as administrator".into(),
            });
        }

        let out = proc::run("scoop", &["update", &candidate.id.native], INSTALL_TIMEOUT).await?;

        if out.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                command: format!("scoop update {}", candidate.id.native),
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
        let out = proc::run("scoop", &["list", &candidate.id.native], QUERY_TIMEOUT).await?;

        if !out.success() {
            return Ok(None);
        }

        // Output: "  package_name  version  bucket  updated"
        // Find the line matching the package name.
        for line in out.stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[0].eq_ignore_ascii_case(&candidate.id.native) {
                return Ok(Some(parts[1].to_string()));
            }
        }

        Ok(None)
    }
}

/// Parse `scoop status` output.
///
/// The text output has sections like:
/// ```text
/// Scoop is up to date.
///
/// These apps are outdated and have updates available:
///
///   firefox 140.0 -> 141.0
///   7zip 23.01 -> 24.07
/// ```
fn parse_scoop_status(stdout: &str) -> Vec<UpdateCandidate> {
    let mut candidates = Vec::new();
    let mut in_outdated_section = false;

    for line in stdout.lines() {
        let line = line.trim();

        if line.contains("outdated") && line.contains("updates") {
            in_outdated_section = true;
            continue;
        }

        // Other sections (missing dependencies, etc.) end our parsing.
        if in_outdated_section && (line.starts_with("These") || line.starts_with("Scoop")) {
            in_outdated_section = false;
            continue;
        }

        if !in_outdated_section || line.is_empty() {
            continue;
        }

        // Parse "package_name old_version -> new_version"
        if let Some((name, old_ver, new_ver)) = parse_outdated_line(line) {
            candidates.push(UpdateCandidate {
                id: PackageId::new(BackendKind::Scoop, name),
                name: name.to_string(),
                installed: Version::parse(old_ver),
                available: Version::parse(new_ver),
                size_bytes: None,
                expected_sha256: None,
            });
        }
    }

    candidates
}

/// Parse `scoop list` output.
///
/// ```text
/// Installed apps:
///
/// Name    Version   Source Updated             Info
/// ----    -------   ------ -------             ----
/// 7zip    24.09     main   2024-09-01 10:00:00
/// git     2.46.0    main   2024-09-02 11:00:00
/// ```
///
/// Only the first two fields are read, so the trailing columns (which are
/// optional and sometimes empty) cannot shift the parse.
fn parse_scoop_list(stdout: &str) -> Vec<InstalledPackage> {
    stdout
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            // Section headings such as "Installed apps:".
            if trimmed.is_empty() || trimmed.ends_with(':') {
                return None;
            }
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() < 2 {
                return None;
            }
            // The header row and the dashed rule beneath it.
            if parts[0] == "Name" || parts[0].chars().all(|c| c == '-') {
                return None;
            }
            let (name, version) = (parts[0], parts[1]);
            if version.is_empty() {
                return None;
            }
            Some(InstalledPackage {
                id: PackageId::new(BackendKind::Scoop, name),
                name: name.to_string(),
                version: version.to_string(),
            })
        })
        .collect()
}

fn parse_outdated_line(line: &str) -> Option<(&str, &str, &str)> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 4 {
        return None;
    }
    // Format: name old_version -> new_version
    if parts[2] != "->" {
        return None;
    }
    Some((parts[0], parts[1], parts[3]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scoop_status() {
        let output = "\
Scoop is up to date.

These apps are outdated and have updates available:

  firefox 140.0 -> 141.0
  7zip 23.01 -> 24.07
";
        let candidates = parse_scoop_status(output);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].id.native, "firefox");
        assert_eq!(candidates[0].installed.raw(), "140.0");
        assert_eq!(candidates[0].available.raw(), "141.0");
        assert_eq!(candidates[1].id.native, "7zip");
    }

    #[test]
    fn empty_output_yields_empty_vec() {
        assert!(parse_scoop_status("").is_empty());
    }

    #[test]
    fn no_outdated_section_yields_empty_vec() {
        let output =
            "Scoop is up to date.\n\nThese apps are outdated and have updates available:\n\n";
        assert!(parse_scoop_status(output).is_empty());
    }

    #[test]
    fn parses_scoop_list() {
        let output = "\
Installed apps:

Name    Version   Source Updated             Info
----    -------   ------ -------             ----
7zip    24.09     main   2024-09-01 10:00:00
git     2.46.0    main   2024-09-02 11:00:00
";
        let installed = parse_scoop_list(output);
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].id.native, "7zip");
        assert_eq!(installed[0].version, "24.09");
        assert_eq!(installed[1].name, "git");
        assert_eq!(installed[1].version, "2.46.0");
    }

    #[test]
    fn scoop_list_ignores_headers_and_blank_lines() {
        assert!(parse_scoop_list("Installed apps:\n\nName  Version\n----  -------\n").is_empty());
        assert!(parse_scoop_list("").is_empty());
    }

    #[test]
    fn scoop_list_tolerates_a_missing_info_column() {
        let output = "\
Name    Version   Source Updated             Info
----    -------   ------ -------             ----
extras  1.0.0     extras 2024-09-01 10:00:00 Held package
plain   2.0.0
";
        let installed = parse_scoop_list(output);
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].version, "1.0.0");
        assert_eq!(installed[1].id.native, "plain");
        assert_eq!(installed[1].version, "2.0.0");
    }

    #[test]
    fn backend_kind_is_correct() {
        let b = ScoopBackend::new();
        assert_eq!(b.kind(), BackendKind::Scoop);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = ScoopBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = ScoopBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::Scoop, "firefox"),
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
