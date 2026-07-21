//! fwupd / LVFS firmware update backend (Linux).
//!
//! Uses `fwupdagent` for machine-readable JSON output (guaranteed stable)
//! and `fwupdmgr` for applying updates.  fwupd downloads firmware from the
//! Linux Vendor Firmware Service (LVFS) at https://fwupd.org.
//!
//! Commands used:
//!   - `fwupdmgr refresh` — download latest metadata from LVFS
//!   - `fwupdagent get-updates --json` — list available firmware updates
//!   - `fwupdmgr update <device-id> -y` — apply firmware update for a device
//!
//! References:
//!   - https://github.com/fwupd/fwupd
//!   - https://manpages.debian.org/bullseye/fwupd/fwupdmgr.1.en.html

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::{ApplyPhase, ApplyProgress, Backend};
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

const REFRESH_TIMEOUT: Duration = Duration::from_secs(60);
const SCAN_TIMEOUT: Duration = Duration::from_secs(30);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);

pub struct FwupdBackend;

impl FwupdBackend {
    pub fn new() -> Self {
        Self
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
        if !cfg!(target_os = "linux") {
            return false;
        }
        proc::exists("fwupdmgr", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(target_os = "linux") {
            return Ok(Vec::new());
        }

        // Refresh metadata first to ensure we have the latest update info.
        let _ = proc::run("fwupdmgr", &["refresh", "-y"], REFRESH_TIMEOUT).await;

        let out = proc::run("fwupdagent", &["get-updates", "--json"], SCAN_TIMEOUT).await?;

        if !out.success() {
            tracing::warn!(stderr = %out.stderr, "fwupdagent get-updates failed");
            return Ok(Vec::new());
        }

        Ok(parse_fwupd_json(&out.stdout))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }

        let device_id = &candidate.id.native;

        let out = proc::run("fwupdmgr", &["update", device_id, "-y"], INSTALL_TIMEOUT).await?;

        if out.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                command: format!("fwupdmgr update {device_id}"),
                code: out.code,
                stderr: if out.stderr.trim().is_empty() {
                    out.stdout
                } else {
                    out.stderr
                },
            })
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
                    message: format!("Updating firmware for {}…", candidate.name),
                    phase: ApplyPhase::Installing,
                })
                .await;
        }

        let result = self.apply(candidate).await;

        if let Some(tx) = &tx {
            if result.is_ok() {
                let _ = tx
                    .send(ApplyProgress {
                        percent: Some(100),
                        message: format!("Firmware update complete for {}", candidate.name),
                        phase: ApplyPhase::Rebooting,
                    })
                    .await;
            }
        }

        result
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        if !cfg!(target_os = "linux") {
            return Ok(None);
        }

        // Query the current firmware version for this device.
        let out = proc::run("fwupdagent", &["get-devices", "--json"], SCAN_TIMEOUT).await?;

        if !out.success() {
            return Ok(None);
        }

        let devices = parse_fwupd_devices(&out.stdout);
        for device in devices {
            if device.instance_id == candidate.id.native {
                return Ok(Some(device.version));
            }
        }

        Ok(None)
    }
}

/// A device from `fwupdagent get-devices --json`.
#[derive(Debug)]
struct FwupdDevice {
    instance_id: String,
    version: String,
}

/// Parse `fwupdagent get-updates --json` output.
///
/// The JSON structure is:
/// ```json
/// {"Devices": [{"Name": "...", "InstanceId": "...", "Releases": [{"Version": "..."}]}]}
/// ```
fn parse_fwupd_json(json: &str) -> Vec<UpdateCandidate> {
    let json = json.trim();
    if json.is_empty() {
        return Vec::new();
    }

    #[derive(serde::Deserialize)]
    struct FwupdUpdate {
        #[serde(rename = "Devices")]
        devices: Vec<UpdateDevice>,
    }

    #[derive(serde::Deserialize)]
    struct UpdateDevice {
        name: String,
        instance_id: Option<String>,
        #[serde(rename = "InstanceId")]
        instance_id_alt: Option<String>,
        releases: Vec<Release>,
    }

    #[derive(serde::Deserialize)]
    struct Release {
        version: String,
    }

    let parsed: FwupdUpdate = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "failed to parse fwupdagent get-updates JSON");
            return Vec::new();
        }
    };

    parsed
        .devices
        .into_iter()
        .filter_map(|d| {
            let instance_id = d.instance_id.or(d.instance_id_alt)?;
            let release = d.releases.into_iter().next()?;
            Some(UpdateCandidate {
                id: PackageId::new(BackendKind::Fwupd, &instance_id),
                name: d.name,
                installed: Version::parse("0.0.0"),
                available: Version::parse(&release.version),
                size_bytes: None,
                expected_sha256: None,
            })
        })
        .collect()
}

/// Parse `fwupdagent get-devices --json` output.
fn parse_fwupd_devices(json: &str) -> Vec<FwupdDevice> {
    let json = json.trim();
    if json.is_empty() {
        return Vec::new();
    }

    #[derive(serde::Deserialize)]
    struct FwupdDevices {
        #[serde(rename = "Devices")]
        devices: Vec<DeviceEntry>,
    }

    #[derive(serde::Deserialize)]
    struct DeviceEntry {
        instance_id: Option<String>,
        #[serde(rename = "InstanceId")]
        instance_id_alt: Option<String>,
        version: Option<String>,
    }

    let parsed: FwupdDevices = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "failed to parse fwupdagent get-devices JSON");
            return Vec::new();
        }
    };

    parsed
        .devices
        .into_iter()
        .filter_map(|d| {
            let instance_id = d.instance_id.or(d.instance_id_alt)?;
            Some(FwupdDevice {
                instance_id,
                version: d.version.unwrap_or_default(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_kind_is_correct() {
        let b = FwupdBackend::new();
        assert_eq!(b.kind(), BackendKind::Fwupd);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = FwupdBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = FwupdBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::Fwupd, "device-id"),
            name: "Firmware".into(),
            installed: Version::parse("1.0.0"),
            available: Version::parse(""),
            size_bytes: None,
            expected_sha256: None,
        };
        let err = backend.apply(&candidate).await.unwrap_err();
        assert!(matches!(err, Error::Verification { .. }));
    }

    #[test]
    fn parses_fwupd_updates_json() {
        let json = r#"{
            "Devices": [
                {
                    "name": "System Firmware",
                    "InstanceId": "b8d8f2c0-...",
                    "releases": [
                        {"version": "1.2.3"}
                    ]
                }
            ]
        }"#;
        let candidates = parse_fwupd_json(json);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id.native, "b8d8f2c0-...");
        assert!(candidates[0].name.contains("System Firmware"));
    }

    #[test]
    fn empty_json_yields_empty_vec() {
        assert!(parse_fwupd_json("").is_empty());
    }

    #[test]
    fn parses_devices_json() {
        let json = r#"{
            "Devices": [
                {
                    "name": "SSD",
                    "InstanceId": "sata-ssd-1",
                    "version": "2.0.0"
                }
            ]
        }"#;
        let devices = parse_fwupd_devices(json);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].instance_id, "sata-ssd-1");
        assert_eq!(devices[0].version, "2.0.0");
    }
}
