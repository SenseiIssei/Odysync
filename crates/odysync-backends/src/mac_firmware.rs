//! macOS firmware and system update backend.
//!
//! Wraps `softwareupdate` — the built-in macOS command-line tool for system
//! and firmware updates, including EFI and SMC firmware updates.
//!
//! Commands used:
//!   - `softwareupdate --list --include-config-data` — list available updates
//!   - `softwareupdate --install <label> --include-config-data --restart --force`
//!
//! On Apple Silicon, `softwareupdate` may prompt for password; use `--stdinpass`
//! or MDM API for unattended operation.
//!
//! References:
//!   - https://keith.github.io/xcode-man-pages/softwareupdate.8.html
//!   - https://eclecticlight.co/2023/09/06/taking-manual-control-of-macos-updates-with-softwareupdate/

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

const SCAN_TIMEOUT: Duration = Duration::from_secs(60);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(3600);

pub struct MacFirmwareBackend;

impl MacFirmwareBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MacFirmwareBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for MacFirmwareBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::MacFirmware
    }

    fn display_name(&self) -> &str {
        "macOS Firmware & System Updates"
    }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "macos")
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(target_os = "macos") {
            return Ok(Vec::new());
        }

        let out = proc::run(
            "softwareupdate",
            &["--list", "--include-config-data"],
            SCAN_TIMEOUT,
        )
        .await?;

        if !out.success() {
            tracing::warn!(stderr = %out.stderr, "softwareupdate --list failed");
            return Ok(Vec::new());
        }

        Ok(parse_softwareupdate_list(&out.stdout))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }

        let label = &candidate.id.native;

        let out = proc::run(
            "softwareupdate",
            &[
                "--install",
                label,
                "--include-config-data",
                "--restart",
                "--force",
            ],
            INSTALL_TIMEOUT,
        )
        .await?;

        if out.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                command: format!("softwareupdate --install {label}"),
                code: out.code,
                stderr: if out.stderr.trim().is_empty() {
                    out.stdout
                } else {
                    out.stderr
                },
            })
        }
    }

    async fn installed_version(&self, _candidate: &UpdateCandidate) -> Result<Option<String>> {
        // softwareupdate doesn't provide a way to query the installed version
        // of a specific update after installation.
        Ok(None)
    }
}

/// Parse `softwareupdate --list --include-config-data` output.
///
/// Output format:
/// ```text
/// Software Update Tool
/// Finding available software
/// Software Update found the following new or updated software:
///    * Label: MacBookAirEFIUpdate2.4-2.4
///         Title: MacBook Air EFI Firmware Update, Version: 2.4, Size: 3817K, Recommended: YES, Action: restart,
///    * Label: ProAppsQTCodecs-1.0
///         Title: ProApps QuickTime codecs, Version: 1.0, Size: 968K, Recommended: YES,
/// ```
fn parse_softwareupdate_list(output: &str) -> Vec<UpdateCandidate> {
    let mut candidates = Vec::new();
    let mut current_label: Option<String> = None;

    for line in output.lines() {
        let line = line.trim();

        // Look for label lines: "* Label: <name>"
        if let Some(label) = extract_label(line) {
            current_label = Some(label);
            continue;
        }

        // Look for title lines with version info.
        if let Some(title) = extract_field(line, "Title:") {
            if let Some(label) = current_label.take() {
                let version = extract_field(line, "Version:").unwrap_or_default();
                let name = title.trim_end_matches(',').to_string();
                let is_firmware =
                    line.contains("Firmware") || line.contains("EFI") || line.contains("SMC");

                candidates.push(UpdateCandidate {
                    id: PackageId::new(BackendKind::MacFirmware, &label),
                    name: if is_firmware {
                        format!("[Firmware] {name}")
                    } else {
                        name
                    },
                    installed: Version::parse("0.0.0"),
                    available: Version::parse(&version),
                    size_bytes: extract_field(line, "Size:").and_then(|s| parse_size(&s)),
                    expected_sha256: None,
                });
            }
        }
    }

    candidates
}

fn extract_label(line: &str) -> Option<String> {
    let pos = line.find("Label:")?;
    let rest = &line[pos + "Label:".len()..];
    let label = rest.trim().trim_end_matches(',').to_string();
    if label.is_empty() {
        None
    } else {
        Some(label)
    }
}

fn extract_field(line: &str, field: &str) -> Option<String> {
    let pos = line.find(field)?;
    let rest = &line[pos + field.len()..];
    let value = rest.split(',').next()?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(k) = s.strip_suffix("K") {
        k.parse::<u64>().ok().map(|n| n * 1024)
    } else if let Some(m) = s.strip_suffix("M") {
        m.parse::<u64>().ok().map(|n| n * 1024 * 1024)
    } else if let Some(g) = s.strip_suffix("G") {
        g.parse::<u64>().ok().map(|n| n * 1024 * 1024 * 1024)
    } else {
        s.parse::<u64>().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_kind_is_correct() {
        let b = MacFirmwareBackend::new();
        assert_eq!(b.kind(), BackendKind::MacFirmware);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = MacFirmwareBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = MacFirmwareBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::MacFirmware, "test-update"),
            name: "Test Update".into(),
            installed: Version::parse("1.0.0"),
            available: Version::parse(""),
            size_bytes: None,
            expected_sha256: None,
        };
        let err = backend.apply(&candidate).await.unwrap_err();
        assert!(matches!(err, Error::Verification { .. }));
    }

    #[test]
    fn parses_softwareupdate_list_output() {
        let output = "\
Software Update Tool
Finding available software
Software Update found the following new or updated software:
   * Label: MacBookAirEFIUpdate2.4-2.4
        Title: MacBook Air EFI Firmware Update, Version: 2.4, Size: 3817K, Recommended: YES, Action: restart,
   * Label: ProAppsQTCodecs-1.0
        Title: ProApps QuickTime codecs, Version: 1.0, Size: 968K, Recommended: YES,
";
        let candidates = parse_softwareupdate_list(output);
        assert_eq!(candidates.len(), 2);

        assert_eq!(candidates[0].id.native, "MacBookAirEFIUpdate2.4-2.4");
        assert!(candidates[0].name.contains("[Firmware]"));
        assert!(candidates[0].name.contains("EFI"));
        assert_eq!(candidates[0].size_bytes, Some(3817 * 1024));

        assert_eq!(candidates[1].id.native, "ProAppsQTCodecs-1.0");
        assert!(!candidates[1].name.contains("[Firmware]"));
    }

    #[test]
    fn empty_output_yields_empty_vec() {
        assert!(parse_softwareupdate_list("").is_empty());
    }

    #[test]
    fn parse_size_handles_suffixes() {
        assert_eq!(parse_size("3817K"), Some(3817 * 1024));
        assert_eq!(parse_size("10M"), Some(10 * 1024 * 1024));
        assert_eq!(parse_size("2G"), Some(2 * 1024 * 1024 * 1024));
        assert_eq!(parse_size("1024"), Some(1024));
        assert_eq!(parse_size("invalid"), None);
    }
}
