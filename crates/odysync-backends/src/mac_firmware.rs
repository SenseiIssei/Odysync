//! macOS firmware updates, and the `softwareupdate` code both macOS sources
//! share.
//!
//! `softwareupdate` lists firmware, macOS itself and Apple's other system
//! software in one table. This source takes the firmware; everything else is
//! [`crate::mac_software_update`].
//!
//! Commands used:
//!   - `softwareupdate --list --include-config-data`: list available updates
//!   - `softwareupdate --install <label> --include-config-data`
//!   - `sw_vers -productVersion`: the running macOS
//!
//! Never with `--restart`: when to restart is the user's decision, and Odysync
//! reports that one is needed. On Apple Silicon `softwareupdate` may want a
//! password; the installation then fails and says so.
//!
//! References:
//!   - <https://keith.github.io/xcode-man-pages/softwareupdate.8.html>
//!   - <https://eclecticlight.co/2026/07/30/take-control-of-updates-using-softwareupdate/>

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::Result;
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

use crate::guard;

/// Apple's server is asked every time, and it takes its time.
const SCAN_TIMEOUT: Duration = Duration::from_secs(300);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(3 * 3600);
const QUERY_TIMEOUT: Duration = Duration::from_secs(30);

/// Lists configuration data as well, which is how Apple ships some firmware.
const CONFIG_DATA: &str = "--include-config-data";

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
        "macOS Firmware Updates"
    }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "macos")
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        scan(Part::Firmware).await
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        apply(Part::Firmware, candidate).await
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        installed_version(Part::Firmware, candidate).await
    }
}

/// Which half of `softwareupdate`'s list a source is responsible for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Part {
    Firmware,
    System,
}

impl Part {
    fn kind(self) -> BackendKind {
        match self {
            Part::Firmware => BackendKind::MacFirmware,
            Part::System => BackendKind::MacSoftwareUpdate,
        }
    }

    fn list_args(self) -> Vec<&'static str> {
        match self {
            Part::Firmware => vec!["--list", CONFIG_DATA],
            Part::System => vec!["--list"],
        }
    }

    fn install_args(self, label: &str) -> Vec<&str> {
        match self {
            Part::Firmware => vec!["--install", label, CONFIG_DATA],
            Part::System => vec!["--install", label],
        }
    }
}

/// List what this part would offer.
pub(crate) async fn scan(part: Part) -> Result<Vec<UpdateCandidate>> {
    if !cfg!(target_os = "macos") {
        return Ok(Vec::new());
    }
    let out = list(part).await?;
    let running = running_macos().await;
    Ok(candidates(part, &out, running.as_deref()))
}

async fn list(part: Part) -> Result<String> {
    let out = proc::run("softwareupdate", &part.list_args(), SCAN_TIMEOUT).await?;
    if !out.success() {
        return Err(guard::failed("softwareupdate --list", out));
    }
    Ok(out.stdout)
}

async fn running_macos() -> Option<String> {
    let out = proc::run("sw_vers", &["-productVersion"], QUERY_TIMEOUT)
        .await
        .ok()?;
    let version = out.stdout.trim();
    (out.success() && !version.is_empty()).then(|| version.to_string())
}

/// Install exactly the label that was approved, if it is still on offer.
pub(crate) async fn apply(part: Part, candidate: &UpdateCandidate) -> Result<()> {
    guard::precheck(candidate)?;

    // The label names one build, but Apple can replace what it stands for.
    let offered = scan(part).await?;
    guard::same_target(
        candidate,
        offered
            .iter()
            .find(|c| c.id == candidate.id)
            .map(|c| c.available.raw()),
    )?;

    let label = candidate.id.native.as_str();
    let out = proc::run("softwareupdate", &part.install_args(label), INSTALL_TIMEOUT).await?;
    if out.success() {
        Ok(())
    } else {
        Err(guard::failed(
            format!("softwareupdate --install {label}"),
            out,
        ))
    }
}

/// `softwareupdate` does not say what is installed. What it no longer lists is.
pub(crate) async fn installed_version(
    part: Part,
    candidate: &UpdateCandidate,
) -> Result<Option<String>> {
    if !cfg!(target_os = "macos") {
        return Ok(None);
    }
    Ok(version_after(candidate, &list(part).await?))
}

/// The version to report after an installation, from a fresh list.
///
/// An update that only takes effect with a restart stays in the list until
/// then, and is reported as not installed yet, which it is not.
fn version_after(candidate: &UpdateCandidate, list: &str) -> Option<String> {
    // Only a list that was really made says anything about what is missing.
    if !list.contains("Finding available software") {
        return None;
    }
    let pending = parse_list(list)
        .iter()
        .any(|e| e.label == candidate.id.native);
    Some(if pending {
        candidate.installed.raw().to_string()
    } else {
        candidate.available.raw().to_string()
    })
}

/// One entry of `softwareupdate --list`.
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    label: String,
    title: String,
    version: String,
    size_bytes: Option<u64>,
}

impl Entry {
    fn is_firmware(&self) -> bool {
        [&self.label, &self.title]
            .iter()
            .any(|t| t.contains("Firmware") || t.contains("EFI") || t.contains("SMC"))
    }

    fn is_macos(&self) -> bool {
        self.title.starts_with("macOS ")
    }
}

/// The entries of one part, as candidates.
fn candidates(part: Part, output: &str, running: Option<&str>) -> Vec<UpdateCandidate> {
    parse_list(output)
        .into_iter()
        .filter(|e| e.is_firmware() == (part == Part::Firmware))
        // A new macOS is an upgrade of the operating system, which nobody asks
        // for by updating their software. The same holds while the running
        // version cannot be read.
        .filter(|e| !e.is_macos() || running.is_some_and(|r| same_major(r, &e.version)))
        .map(|e| UpdateCandidate {
            id: PackageId::new(part.kind(), e.label.as_str()),
            // `softwareupdate` decides what applies to this Mac and does not
            // say what it replaces, so anything but macOS itself is compared
            // the way Windows updates are: missing, or not.
            installed: Version::parse(match running {
                Some(r) if e.is_macos() => r,
                _ => "0",
            }),
            available: Version::parse(&e.version),
            name: e.title,
            size_bytes: e.size_bytes,
            expected_sha256: None,
        })
        .collect()
}

/// Whether two versions belong to the same macOS. Before 11 the second number
/// was what changed every year.
fn same_major(a: &str, b: &str) -> bool {
    let major = |v: &str| -> Option<(u64, u64)> {
        let mut parts = v.trim().split('.');
        let first = parts.next()?.parse().ok()?;
        let second = match first {
            10 => parts.next()?.parse().ok()?,
            _ => 0,
        };
        Some((first, second))
    };
    matches!((major(a), major(b)), (Some(x), Some(y)) if x == y)
}

/// Parse `softwareupdate --list`.
///
/// ```text
/// Software Update found the following new or updated software:
/// * Label: macOS Tahoe 26.6-25G72
///         Title: macOS Tahoe 26.6, Version: 26.6, Size: 7721694KiB, Recommended: YES, Action: restart,
/// ```
fn parse_list(output: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut label: Option<String> = None;

    for line in output.lines() {
        let line = line.trim();
        if let Some(rest) = line
            .trim_start_matches('*')
            .trim_start()
            .strip_prefix("Label:")
        {
            let rest = rest.trim().trim_end_matches(',').to_string();
            label = (!rest.is_empty()).then_some(rest);
            continue;
        }
        if let Some(title) = extract_field(line, "Title:")
            && let Some(label) = label.take()
        {
            entries.push(Entry {
                label,
                title,
                version: extract_field(line, "Version:").unwrap_or_default(),
                size_bytes: extract_field(line, "Size:").and_then(|s| parse_size(&s)),
            });
        }
    }

    entries
}

fn extract_field(line: &str, field: &str) -> Option<String> {
    let (_, rest) = line.split_once(field)?;
    let value = rest.split(',').next()?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// `3817K` from older systems, `23593KiB` from current ones.
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    let (number, unit) = s.split_at(digits);
    let factor = match unit {
        "" => 1,
        "K" | "KiB" => 1024,
        "M" | "MiB" => 1024 * 1024,
        "G" | "GiB" => 1024 * 1024 * 1024,
        _ => return None,
    };
    number.parse::<u64>().ok()?.checked_mul(factor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang_backends::assert_refuses_bad_input;

    /// The three entries at the end are from
    /// https://eclecticlight.co/2026/07/30/take-control-of-updates-using-softwareupdate/,
    /// the two at the start from the older format of the manual page.
    const LIST: &str = "\
Software Update Tool

Finding available software
Software Update found the following new or updated software:
   * Label: MacBookAirEFIUpdate2.4-2.4
        Title: MacBook Air EFI Firmware Update, Version: 2.4, Size: 3817K, Recommended: YES, Action: restart,
   * Label: ProAppsQTCodecs-1.0
        Title: ProApps QuickTime codecs, Version: 1.0, Size: 968K, Recommended: YES,
* Label: XProtectPayloads_10_15-157
\tTitle: XProtectPayloads, Version: 157, Size: 23593KiB, Recommended: YES,
* Label: XProtectPlistConfigData_10_15-5353
\tTitle: XProtectPlistConfigData, Version: 5353, Size: 1281KiB, Recommended: YES,
* Label: macOS Tahoe 26.6-25G72
\tTitle: macOS Tahoe 26.6, Version: 26.6, Size: 7721694KiB, Recommended: YES, Action: restart,
";

    fn ids(list: &[UpdateCandidate]) -> Vec<&str> {
        list.iter().map(|c| c.id.native.as_str()).collect()
    }

    #[test]
    fn every_entry_belongs_to_exactly_one_source() {
        for text in [LIST.to_string(), LIST.replace('\n', "\r\n")] {
            assert_eq!(parse_list(&text).len(), 5);
            let firmware = candidates(Part::Firmware, &text, Some("26.5.1"));
            let system = candidates(Part::System, &text, Some("26.5.1"));
            assert_eq!(ids(&firmware), ["MacBookAirEFIUpdate2.4-2.4"]);
            assert_eq!(
                ids(&system),
                [
                    "ProAppsQTCodecs-1.0",
                    "XProtectPayloads_10_15-157",
                    "XProtectPlistConfigData_10_15-5353",
                    "macOS Tahoe 26.6-25G72",
                ]
            );
            assert_eq!(firmware[0].id.backend, BackendKind::MacFirmware);
            assert_eq!(system[0].id.backend, BackendKind::MacSoftwareUpdate);
            assert_eq!(firmware[0].size_bytes, Some(3817 * 1024));
            assert_eq!(system[1].size_bytes, Some(23593 * 1024));
            // macOS is compared with the running version.
            assert_eq!(system[3].installed.raw(), "26.5.1");
            assert_eq!(system[3].available.raw(), "26.6");
        }
    }

    #[test]
    fn a_new_macos_is_not_an_ordinary_update() {
        for running in [Some("15.7.1"), Some("27.0"), Some(""), None] {
            let system = candidates(Part::System, LIST, running);
            assert_eq!(system.len(), 3, "{running:?}");
            assert!(!system.iter().any(|c| c.name.starts_with("macOS")));
        }
        assert!(same_major("15.1", "15.7.2"));
        assert!(!same_major("15.1", "26.0"));
        // 10.14 to 10.15 was a new macOS as well.
        assert!(!same_major("10.14.6", "10.15.7"));
        assert!(same_major("10.15.1", "10.15.7"));
        assert!(!same_major("x", "x"));
    }

    #[test]
    fn what_is_no_longer_listed_is_installed() {
        let firmware = candidates(Part::Firmware, LIST, Some("26.5.1")).remove(0);
        // Still listed, for example because a restart is missing.
        assert_eq!(version_after(&firmware, LIST).as_deref(), Some("0"));
        let done = "Software Update Tool\n\nFinding available software\n";
        assert_eq!(version_after(&firmware, done).as_deref(), Some("2.4"));
        // No list at all proves nothing.
        assert_eq!(version_after(&firmware, ""), None);
        assert_eq!(version_after(&firmware, "Can't connect"), None);
    }

    #[test]
    fn nothing_is_restarted_or_forced() {
        for part in [Part::Firmware, Part::System] {
            let args = part.install_args("Label-1.0");
            assert_eq!(args[..2], ["--install", "Label-1.0"]);
            assert!(!args.contains(&"--restart"));
            assert!(!args.contains(&"--force"));
            assert!(!args.contains(&"--all"));
        }
    }

    #[test]
    fn a_moved_target_is_refused() {
        let firmware = candidates(Part::Firmware, LIST, None).remove(0);
        assert!(guard::same_target(&firmware, Some("2.4")).is_ok());
        assert!(guard::same_target(&firmware, Some("2.5")).is_err());
        assert!(guard::same_target(&firmware, None).is_err());
    }

    #[test]
    fn backend_kind_is_correct() {
        let b = MacFirmwareBackend::new();
        assert_eq!(b.kind(), BackendKind::MacFirmware);
        assert!(!b.display_name().is_empty());
    }

    #[test]
    fn empty_output_yields_empty_vec() {
        assert!(parse_list("").is_empty());
        assert!(parse_list("No new software available.\n").is_empty());
    }

    #[test]
    fn parse_size_handles_suffixes() {
        assert_eq!(parse_size("3817K"), Some(3817 * 1024));
        assert_eq!(parse_size("3817KiB"), Some(3817 * 1024));
        assert_eq!(parse_size("10M"), Some(10 * 1024 * 1024));
        assert_eq!(parse_size("2G"), Some(2 * 1024 * 1024 * 1024));
        assert_eq!(parse_size("1024"), Some(1024));
        assert_eq!(parse_size("invalid"), None);
        assert_eq!(parse_size("5 bananas"), None);
    }

    #[tokio::test]
    async fn apply_refuses_options_and_unknown_targets() {
        assert_refuses_bad_input(&MacFirmwareBackend::new(), "Label-1.0", None).await;
    }
}
