//! fwupd / LVFS firmware update backend (Linux).
//!
//! fwupd downloads firmware from the Linux Vendor Firmware Service (LVFS) at
//! <https://fwupd.org> and checks its signature. A flash that went wrong is
//! not something to try again on its own, so nothing here is reported as
//! retryable once the installation has started.
//!
//! Commands used:
//!   - `fwupdmgr refresh`: download latest metadata from LVFS
//!   - `fwupdmgr get-updates --json`: available firmware (exit 2 = nothing to do)
//!   - `fwupdmgr get-devices --json`: installed versions
//!   - `fwupdmgr install <device-id> <version>`: install one exact release
//!
//! References:
//!   - <https://github.com/fwupd/fwupd/blob/main/src/fwupdmgr.md>
//!   - <https://github.com/fwupd/fwupd/blob/1.9.20/src/fu-util.c>

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::{ApplyPhase, ApplyProgress, Backend};
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

use crate::guard;

const REFRESH_TIMEOUT: Duration = Duration::from_secs(120);
const SCAN_TIMEOUT: Duration = Duration::from_secs(60);
/// Killing a flash halfway is worse than waiting, so this is generous.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// "Commands that have no actions but were successfully executed."
const EXIT_NOTHING_TO_DO: i32 = 2;

pub struct FwupdBackend;

impl FwupdBackend {
    pub fn new() -> Self {
        Self
    }

    async fn devices(command: &str) -> Result<Vec<FwupdDevice>> {
        let out = proc::run(
            "fwupdmgr",
            // Without the metadata check fwupdmgr may stop to ask whether it
            // should refresh first.
            &[command, "--json", "--no-metadata-check"],
            SCAN_TIMEOUT,
        )
        .await?;
        if out.code == EXIT_NOTHING_TO_DO {
            return Ok(Vec::new());
        }
        if !out.success() {
            return Err(guard::failed(format!("fwupdmgr {command}"), out));
        }
        parse_devices(&out.stdout)
    }
}

impl Default for FwupdBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for FwupdBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Fwupd
    }

    fn display_name(&self) -> &str {
        "fwupd (Linux Firmware)"
    }

    async fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && proc::exists("fwupdmgr", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        // Refresh metadata first. Not with `-y`: that would also answer the
        // question whether a disabled remote should be enabled.
        let _ = proc::run(
            "fwupdmgr",
            &["refresh", "--no-remote-check"],
            REFRESH_TIMEOUT,
        )
        .await;

        Ok(Self::devices("get-updates")
            .await?
            .iter()
            .filter_map(FwupdDevice::candidate)
            .collect())
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        guard::precheck(candidate)?;

        // `fwupdmgr install` allows older releases and reinstallation, so both
        // ends are read again first.
        let offered = Self::devices("get-updates").await?;
        guard::same_target(
            candidate,
            offered
                .iter()
                .find(|d| d.device_id == candidate.id.native)
                .and_then(FwupdDevice::newest_release),
        )?;
        guard::still_older(
            candidate,
            self.installed_version(candidate).await?.as_deref(),
        )?;

        let args = install_args(&candidate.id.native, candidate.available.raw());
        match proc::run("fwupdmgr", &args, INSTALL_TIMEOUT).await {
            Ok(out) if out.success() => Ok(()),
            Ok(out) => Err(never_retry(
                candidate,
                guard::failed(format!("fwupdmgr install {}", candidate.id.native), out),
            )),
            Err(e) => Err(never_retry(candidate, e)),
        }
    }

    async fn apply_with_progress(
        &self,
        candidate: &UpdateCandidate,
        tx: Option<tokio::sync::mpsc::Sender<ApplyProgress>>,
    ) -> Result<()> {
        if let Some(tx) = &tx {
            let _ = tx
                .send(ApplyProgress {
                    percent: None,
                    message: format!("Updating firmware for {}", candidate.name),
                    phase: ApplyPhase::Installing,
                })
                .await;
        }

        let result = self.apply(candidate).await;

        if let Some(tx) = &tx
            && result.is_ok()
        {
            let _ = tx
                .send(ApplyProgress {
                    percent: Some(100),
                    message: format!("Firmware update complete for {}", candidate.name),
                    phase: ApplyPhase::Rebooting,
                })
                .await;
        }

        result
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        Ok(Self::devices("get-devices")
            .await?
            .into_iter()
            .find(|d| d.device_id == candidate.id.native)
            .and_then(|d| d.version))
    }
}

/// `--no-reboot-check` belongs to `--assume-yes`: without it, "yes to all
/// questions" includes "restart now?".
fn install_args<'a>(device_id: &'a str, version: &'a str) -> [&'a str; 7] {
    [
        "install",
        device_id,
        version,
        "--assume-yes",
        "--no-reboot-check",
        "--no-unreported-check",
        "--no-metadata-check",
    ]
}

/// Turn a failure the runner would retry into one it will not.
fn never_retry(candidate: &UpdateCandidate, error: Error) -> Error {
    if error.is_retryable() {
        guard::refused(
            candidate,
            format!("{error}. A firmware update is not repeated; check the device first"),
        )
    } else {
        error
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct FwupdDevice {
    name: Option<String>,
    device_id: String,
    version: Option<String>,
    #[serde(default)]
    releases: Vec<FwupdRelease>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct FwupdRelease {
    version: Option<String>,
    size: Option<u64>,
}

impl FwupdDevice {
    /// The release `fwupdmgr update` would pick. fwupd lists the newest first;
    /// where our own ordering disagrees, there is no safe answer.
    fn newest_release(&self) -> Option<&str> {
        let versions: Vec<&str> = self
            .releases
            .iter()
            .filter_map(|r| r.version.as_deref())
            .collect();
        let newest = guard::newest(versions.iter().copied(), Version::parse)?;
        (versions.first() == Some(&newest)).then_some(newest)
    }

    fn candidate(&self) -> Option<UpdateCandidate> {
        let available = self.newest_release()?;
        let size_bytes = self
            .releases
            .iter()
            .find(|r| r.version.as_deref() == Some(available))
            .and_then(|r| r.size);
        Some(UpdateCandidate {
            id: PackageId::new(BackendKind::Fwupd, self.device_id.as_str()),
            name: self.name.clone().unwrap_or_else(|| self.device_id.clone()),
            installed: Version::parse(self.version.as_deref().unwrap_or("")),
            available: Version::parse(available),
            size_bytes,
            expected_sha256: None,
        })
    }
}

/// Parse `get-updates --json` and `get-devices --json`; the first adds the
/// releases. Entries without a device id are not something to act on.
fn parse_devices(json: &str) -> Result<Vec<FwupdDevice>> {
    #[derive(serde::Deserialize)]
    struct Root {
        #[serde(rename = "Devices", default)]
        devices: Vec<serde_json::Value>,
    }
    let root: Root = serde_json::from_str(json.trim())
        .map_err(|e| Error::parse("fwupdmgr --json", e.to_string()))?;
    Ok(root
        .devices
        .into_iter()
        .filter_map(|d| serde_json::from_value(d).ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang_backends::assert_refuses_bad_input;

    /// Keys as `fwupd_device_to_json_full` writes them
    /// (https://github.com/fwupd/fwupd/blob/1.9.20/libfwupd/fwupd-device.c);
    /// the hub is the example of
    /// https://developers.redhat.com/articles/2023/10/06/use-fwupd-deploy-linux-firmware-updates-and-more
    const UPDATES: &str = r#"{
  "Devices" : [
    {
      "Name" : "System Firmware",
      "DeviceId" : "a45df35ac0e948ee180fe216a5f703f32dda163f",
      "InstanceIds" : ["UEFI\\RES_{B4BEFE56-1B4C-5A1C-B3F0-0E0A4C0E6C7D}"],
      "Guid" : ["b4befe56-1b4c-5a1c-b3f0-0e0a4c0e6c7d"],
      "Plugin" : "uefi_capsule",
      "Flags" : ["internal", "updatable", "require-ac", "supported"],
      "Vendor" : "LENOVO",
      "Version" : "0.1.39",
      "VersionFormat" : "triplet",
      "VersionRaw" : 65575,
      "Releases" : [
        {"AppstreamId" : "com.lenovo.ThinkPadN2JET.firmware", "RemoteId" : "lvfs", "Name" : "ThinkPad System Update", "Version" : "0.1.41", "Size" : 12345678, "Urgency" : "high"},
        {"AppstreamId" : "com.lenovo.ThinkPadN2JET.firmware", "RemoteId" : "lvfs", "Version" : "0.1.40", "Size" : 12345670, "Urgency" : "high"}
      ]
    },
    {
      "Name" : "USB2.0 Hub",
      "DeviceId" : "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567",
      "Guid" : ["5e09ae3e-3d6f-5e8b-b4b5-5ff0c3f6b8a1"],
      "Summary" : "USB 3.x hub",
      "Vendor" : "VIA Labs, Inc.",
      "Version" : "51.63",
      "VersionFormat" : "bcd"
    },
    {"Name" : "No id"}
  ]
}"#;

    #[test]
    fn updates_carry_the_installed_and_the_newest_version() {
        for text in [UPDATES.to_string(), UPDATES.replace('\n', "\r\n")] {
            let devices = parse_devices(&text).unwrap();
            assert_eq!(devices.len(), 2);
            let found: Vec<UpdateCandidate> =
                devices.iter().filter_map(FwupdDevice::candidate).collect();
            assert_eq!(found.len(), 1);
            assert_eq!(
                found[0].id.native,
                "a45df35ac0e948ee180fe216a5f703f32dda163f"
            );
            assert_eq!(found[0].name, "System Firmware");
            assert_eq!(found[0].installed.raw(), "0.1.39");
            assert_eq!(found[0].available.raw(), "0.1.41");
            assert_eq!(found[0].size_bytes, Some(12345678));
            assert!(found[0].installed.is_upgrade_to(&found[0].available));
        }
    }

    #[test]
    fn a_release_list_in_an_order_we_do_not_understand_is_left_alone() {
        let json = r#"{"Devices":[{"Name":"x","DeviceId":"1","Version":"1.0",
            "Releases":[{"Version":"1.1"},{"Version":"1.2"}]}]}"#;
        assert_eq!(parse_devices(json).unwrap()[0].newest_release(), None);
        let json = r#"{"Devices":[{"Name":"x","DeviceId":"1","Version":"1.0",
            "Releases":[{"Version":"banana"},{"Version":"1.2"}]}]}"#;
        assert_eq!(parse_devices(json).unwrap()[0].newest_release(), None);
    }

    #[test]
    fn broken_json_is_an_error_and_an_empty_list_is_not() {
        assert!(parse_devices("").is_err());
        assert!(parse_devices("No updates available").is_err());
        assert!(parse_devices(r#"{"Devices":[]}"#).unwrap().is_empty());
    }

    #[test]
    fn the_install_names_device_and_version_and_never_restarts() {
        let args = install_args("a45d", "0.1.41");
        assert_eq!(args[..3], ["install", "a45d", "0.1.41"]);
        assert!(args.contains(&"--no-reboot-check"));
        assert!(!args.iter().any(|a| {
            a.contains("force") || a.contains("allow") || a.contains("no-safety-check")
        }));
    }

    #[test]
    fn a_flash_is_never_retried() {
        let candidate = parse_devices(UPDATES).unwrap()[0].candidate().unwrap();
        let errors = [
            Error::transient("fwupd", "the daemon went away", 0),
            Error::CommandTimeout {
                command: "fwupdmgr".into(),
                seconds: 3600,
            },
            Error::CommandFailed {
                command: "fwupdmgr".into(),
                code: 143,
                stderr: String::new(),
            },
            Error::Io(std::io::ErrorKind::Interrupted.into()),
        ];
        assert!(errors.iter().any(Error::is_retryable));
        for error in errors {
            assert!(!never_retry(&candidate, error).is_retryable());
        }
        let failed = Error::CommandFailed {
            command: "fwupdmgr".into(),
            code: 1,
            stderr: "device not found".into(),
        };
        assert!(matches!(
            never_retry(&candidate, failed),
            Error::CommandFailed { code: 1, .. }
        ));
    }

    #[test]
    fn a_moved_target_is_refused() {
        let candidate = parse_devices(UPDATES).unwrap()[0].candidate().unwrap();
        assert!(guard::same_target(&candidate, Some("0.1.41")).is_ok());
        assert!(guard::same_target(&candidate, Some("0.1.42")).is_err());
        assert!(guard::still_older(&candidate, Some("0.1.41")).is_err());
    }

    #[test]
    fn backend_kind_is_correct() {
        let b = FwupdBackend::new();
        assert_eq!(b.kind(), BackendKind::Fwupd);
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_options_and_unknown_targets() {
        assert_refuses_bad_input(&FwupdBackend::new(), "a45df35ac0e9", None).await;
    }
}
