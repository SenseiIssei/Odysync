//! Lenovo System Update backend.
//!
//! Wraps Lenovo's `tvsu.exe` (System Update) and `SUHelper.exe` (Commercial
//! Vantage helper) for driver, BIOS, and firmware updates on Lenovo PCs.
//!
//! Commands used:
//!   - `tvsu.exe /CM -search A -action LIST -exporttowmi` — scan for updates
//!   - `tvsu.exe /CM -search A -action INSTALL -includerebootpackages 3 -noreboot -nolicense`
//!   - `SUHelper.exe -autoupdate -packagetype 1,2,3,4,5 -noreboot`
//!
//! References:
//!   - https://docs.lenovocdrt.com/guides/sus/su_dg/su_dg_ch5/
//!   - https://docs.lenovocdrt.com/guides/lcv/suhelper/

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;

use crate::oem;

const SCAN_TIMEOUT: Duration = Duration::from_secs(120);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(1800);

pub struct LenovoSystemUpdateBackend {
    tvsu_path: std::path::PathBuf,
    suhelper_path: Option<std::path::PathBuf>,
}

impl LenovoSystemUpdateBackend {
    pub fn new() -> Self {
        let tool_path = oem::oem_tool_path(oem::OemManufacturer::Lenovo)
            .unwrap_or_else(|| std::path::PathBuf::from("tvsu.exe"));

        // Check if the path is tvsu.exe or SUHelper.exe
        let (tvsu_path, suhelper_path) = if tool_path
            .file_name()
            .map(|n| n == "SUHelper.exe")
            .unwrap_or(false)
        {
            (
                std::path::PathBuf::from(
                    r"C:\Program Files (x86)\Lenovo\System Update\tvsu.exe",
                ),
                Some(tool_path),
            )
        } else {
            (tool_path, None)
        };

        Self {
            tvsu_path,
            suhelper_path,
        }
    }
}

impl Default for LenovoSystemUpdateBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for LenovoSystemUpdateBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::LenovoSystemUpdate
    }

    fn display_name(&self) -> &str {
        "Lenovo System Update"
    }

    async fn is_available(&self) -> bool {
        if !cfg!(windows) {
            return false;
        }
        let oem_manufacturer = oem::OemManufacturer::detect().await;
        if oem_manufacturer != oem::OemManufacturer::Lenovo {
            return false;
        }
        oem::tool_exists(self.tvsu_path.to_str().unwrap_or(""))
            || self
                .suhelper_path
                .as_ref()
                .map(|p| oem::tool_exists(p.to_str().unwrap_or("")))
                .unwrap_or(false)
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(windows) {
            return Ok(Vec::new());
        }

        if !oem::tool_exists(self.tvsu_path.to_str().unwrap_or("")) {
            tracing::warn!("tvsu.exe not found; cannot scan for Lenovo updates");
            return Ok(Vec::new());
        }

        let out = proc::run(
            self.tvsu_path.to_str().unwrap_or("tvsu.exe"),
            &["/CM", "-search", "A", "-action", "LIST", "-exporttowmi"],
            SCAN_TIMEOUT,
        )
        .await?;

        if !out.success() {
            tracing::warn!(stderr = %out.stderr, "tvsu scan failed");
            return Ok(Vec::new());
        }

        // Lenovo System Update outputs results to WMI.  Query WMI for the
        // list of available updates.
        let wmi_out = proc::powershell(
            r#"
            Get-WmiObject -Namespace root\Lenovo -Class Lenovo_UpdatePackage |
                Select-Object PackageID, Name, Version, PackageType |
                ConvertTo-Json
            "#,
            Duration::from_secs(30),
        )
        .await?;

        if !wmi_out.success() {
            return Ok(Vec::new());
        }

        Ok(parse_lenovo_wmi_output(&wmi_out.stdout))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }

        // Try SUHelper first if available (lighter weight).
        if let Some(suhelper) = &self.suhelper_path {
            if oem::tool_exists(suhelper.to_str().unwrap_or("")) {
                let out = proc::run(
                    suhelper.to_str().unwrap_or("SUHelper.exe"),
                    &[
                        "-autoupdate",
                        "-include",
                        &candidate.id.native,
                        "-noreboot",
                    ],
                    INSTALL_TIMEOUT,
                )
                .await?;

                if out.success() {
                    return Ok(());
                }
                // Fall through to tvsu on failure.
            }
        }

        let out = proc::run(
            self.tvsu_path.to_str().unwrap_or("tvsu.exe"),
            &[
                "/CM",
                "-search",
                "A",
                "-action",
                "INSTALL",
                "-includerebootpackages",
                "3",
                "-noreboot",
                "-nolicense",
            ],
            INSTALL_TIMEOUT,
        )
        .await?;

        if out.success() {
            Ok(())
        } else {
            Err(Error::CommandFailed {
                command: "tvsu /action INSTALL".into(),
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
        // Lenovo System Update doesn't expose per-package installed versions
        // via CLI in a easily parseable way.
        Ok(None)
    }
}

/// Parse Lenovo WMI JSON output for update packages.
fn parse_lenovo_wmi_output(json: &str) -> Vec<UpdateCandidate> {
    let json = json.trim();
    if json.is_empty() {
        return Vec::new();
    }

    // WMI JSON can be a single object or an array.
    #[derive(serde::Deserialize)]
    struct WmiUpdate {
        #[serde(rename = "PackageID")]
        package_id: String,
        #[serde(rename = "Name")]
        name: String,
        #[serde(rename = "Version")]
        version: String,
    }

    let updates: Vec<WmiUpdate> = if json.starts_with('[') {
        match serde_json::from_str(json) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse Lenovo WMI JSON array");
                return Vec::new();
            }
        }
    } else {
        match serde_json::from_str::<WmiUpdate>(json) {
            Ok(u) => vec![u],
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse Lenovo WMI JSON object");
                return Vec::new();
            }
        }
    };

    updates
        .into_iter()
        .map(|u| UpdateCandidate {
            id: PackageId::new(BackendKind::LenovoSystemUpdate, &u.package_id),
            name: u.name,
            installed: Version::parse("0.0.0"),
            available: Version::parse(&u.version),
            size_bytes: None,
            expected_sha256: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_kind_is_correct() {
        let b = LenovoSystemUpdateBackend::new();
        assert_eq!(b.kind(), BackendKind::LenovoSystemUpdate);
    }

    #[test]
    fn display_name_is_non_empty() {
        let b = LenovoSystemUpdateBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = LenovoSystemUpdateBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::LenovoSystemUpdate, "pkg123"),
            name: "Lenovo Driver".into(),
            installed: Version::parse("1.0.0"),
            available: Version::parse(""),
            size_bytes: None,
            expected_sha256: None,
        };
        let err = backend.apply(&candidate).await.unwrap_err();
        assert!(matches!(err, Error::Verification { .. }));
    }

    #[test]
    fn parses_wmi_json_array() {
        let json = r#"[
            {"PackageID":"n1hga14w","Name":"Intel Wireless Driver","Version":"23.0.0"},
            {"PackageID":"n1hga15w","Name":"Realtek Audio Driver","Version":"6.0.9500"}
        ]"#;
        let candidates = parse_lenovo_wmi_output(json);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].id.native, "n1hga14w");
        assert!(candidates[0].name.contains("Wireless"));
    }

    #[test]
    fn parses_wmi_json_single_object() {
        let json = r#"{"PackageID":"n1hga14w","Name":"Intel Wireless Driver","Version":"23.0.0"}"#;
        let candidates = parse_lenovo_wmi_output(json);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id.native, "n1hga14w");
    }

    #[test]
    fn empty_json_yields_empty_vec() {
        assert!(parse_lenovo_wmi_output("").is_empty());
    }
}
