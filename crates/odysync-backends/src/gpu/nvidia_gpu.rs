//! NVIDIA GPU drivers, straight from NVIDIA.
//!
//! The latest driver for each GPU is looked up by its PCI device id on the
//! same NVIDIA service the NVIDIA App and GeForce Experience query, so the
//! answer is NVIDIA's own and specific to the card in the machine. Nothing
//! here depends on winget, which previously supplied this backend — and only
//! knew a single generic `Nvidia.GeForceDriver` package for every card.
//!
//! ## What is accepted
//!
//! * **WHQL only.** A driver the service marks as beta or not WHQL-certified
//!   is not offered, whatever the stable-only policy says.
//! * **Downloaded from `*.download.nvidia.com` over HTTPS**, and refused
//!   unless its Authenticode signature is valid and issued to NVIDIA
//!   Corporation — the same chain as every other installer Odysync runs
//!   (see [`crate::direct::windows_apply::lock_and_verify`]).
//! * **Never a clean install.** The installer runs with `-s -noreboot`, which
//!   upgrades in place and keeps the user's NVIDIA settings; `-clean` would
//!   reset them and is deliberately not used.

use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::version::Version;

use super::{GpuAdapter, GpuVendor, enumerate_display_adapters};

/// Native id of the one package this backend manages: the display driver.
const PACKAGE_ID: &str = "geforce-driver";

/// The signer every NVIDIA driver package carries.
#[cfg(windows)]
const NVIDIA_SIGNER: &str = "NVIDIA Corporation";

/// A full driver package is around 1 GB, and the installer then reconfigures
/// every display; both take a while on a slow machine.
#[cfg(windows)]
const INSTALL_TIMEOUT: Duration = Duration::from_secs(45 * 60);

pub struct NvidiaGpuBackend;

impl NvidiaGpuBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NvidiaGpuBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// The NVIDIA driver version Windows reports, in NVIDIA's own spelling.
///
/// Windows records the driver as `32.0.16.1692`; NVIDIA calls the same driver
/// `616.92`. The NVIDIA number is the last five digits of the Windows one with
/// the dot before the final two — a convention NVIDIA has kept since the
/// R300 drivers. Anything that does not fit the shape returns `None` rather
/// than a guess, which the policy engine then treats as an unknown version.
pub fn nvidia_version_from_windows(driver_version: &str) -> Option<String> {
    let digits: String = driver_version
        .trim()
        .chars()
        .filter(|c| *c != '.')
        .collect();
    if digits.len() < 5 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let tail = &digits[digits.len() - 5..];
    let major: u32 = tail[..3].parse().ok()?;
    Some(format!("{major}.{}", &tail[3..]))
}

/// One driver offered by NVIDIA's lookup service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NvidiaDriver {
    pub name: String,
    pub version: String,
    pub url: String,
    pub details_url: Option<String>,
}

/// Read the lookup service's answer, refusing anything that is not a
/// WHQL-certified, non-beta driver hosted by NVIDIA over HTTPS.
pub fn parse_lookup(json: &serde_json::Value) -> Option<NvidiaDriver> {
    let attrs = json.get("DriverAttributes")?;
    let text = |k: &str| {
        attrs.get(k).and_then(|v| match v {
            serde_json::Value::String(s) => Some(s.trim().to_string()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    };
    if text("IsWHQL").as_deref() != Some("1") || text("IsBeta").as_deref() == Some("1") {
        return None;
    }
    let version = text("Version").filter(|v| !v.is_empty())?;
    // `DownloadURLAdmin` is the standard package; `DownloadURL` is the variant
    // the NVIDIA App fetches. Either is acceptable once its host is checked.
    let url = text("DownloadURLAdmin")
        .filter(|u| is_nvidia_download(u))
        .or_else(|| text("DownloadURL").filter(|u| is_nvidia_download(u)))?;
    Some(NvidiaDriver {
        name: text("Name").unwrap_or_else(|| "NVIDIA display driver".into()),
        version,
        url,
        details_url: text("DetailsURL").filter(|u| u.starts_with("https://")),
    })
}

/// HTTPS, on an NVIDIA download host, and an `.exe`.
pub fn is_nvidia_download(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("").to_ascii_lowercase();
    (host == "download.nvidia.com" || host.ends_with(".download.nvidia.com"))
        && url.to_ascii_lowercase().ends_with(".exe")
}

/// The lookup URL for one GPU.
///
/// The query is a JSON object in the path, which is how NVIDIA's service is
/// built. `dch=1` asks for a DCH driver, the only kind NVIDIA has shipped for
/// current GPUs since 2023; `iLp` selects the notebook package on a laptop.
pub fn lookup_url(device_id: &str, os_build: &str, laptop: bool) -> String {
    let query = serde_json::json!({
        "dIDa": [format!("{}_10DE", device_id.to_ascii_uppercase())],
        "osC": "10.0",
        "osB": os_build,
        "is6": "1",
        "lg": "1033",
        "iLp": if laptop { "1" } else { "0" },
        "prvMd": "0",
        "gcV": "3.28.0.417",
        "gIsB": "0",
        "dch": "1",
        "upCRD": "0",
        "isCRD": "0",
    });
    let encoded: String = query
        .to_string()
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect();
    format!(
        "https://gfwsl.geforce.com/nvidia_web_services/controller.gfeclientcontent.NG.php/\
         com.nvidia.services.GFEClientContent_NG.getDispDrvrByDevid/{encoded}"
    )
}

/// The Windows build number, e.g. `26100`.
#[cfg(windows)]
fn os_build() -> String {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    winreg::RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .and_then(|k| k.get_value::<String, _>("CurrentBuildNumber"))
        .unwrap_or_else(|_| "26100".into())
}

async fn nvidia_adapters() -> Vec<GpuAdapter> {
    let mut adapters: Vec<GpuAdapter> = enumerate_display_adapters()
        .await
        .into_iter()
        .filter(|a| a.vendor == GpuVendor::Nvidia)
        .collect();
    // Two identical cards share one driver.
    adapters.dedup_by(|a, b| a.device_id == b.device_id);
    adapters
}

#[cfg(windows)]
async fn lookup(adapter: &GpuAdapter) -> Result<Option<NvidiaDriver>> {
    let client = crate::direct::http::client(None, Duration::from_secs(60))?;
    let laptop = odysync_core::platform::has_system_battery();
    let url = lookup_url(&adapter.device_id, &os_build(), laptop);
    let json = crate::direct::http::get_json(&client, &url).await?;
    Ok(parse_lookup(&json))
}

#[async_trait]
impl Backend for NvidiaGpuBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::NvidiaGpu
    }

    fn display_name(&self) -> &str {
        "NVIDIA GPU Driver"
    }

    async fn is_available(&self) -> bool {
        cfg!(windows) && !nvidia_adapters().await.is_empty()
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        #[cfg(windows)]
        {
            // One driver package serves every NVIDIA GPU in the machine, so
            // the first card that gets an answer decides it.
            for adapter in nvidia_adapters().await {
                let Some(driver) = lookup(&adapter).await? else {
                    continue;
                };
                let installed =
                    nvidia_version_from_windows(&adapter.driver_version).unwrap_or_default();
                return Ok(vec![UpdateCandidate {
                    id: PackageId::new(BackendKind::NvidiaGpu, PACKAGE_ID)
                        .with_alias("nvidia-driver"),
                    name: format!("{} for {}", driver.name, adapter.name),
                    installed: Version::parse(&installed),
                    available: Version::parse(&driver.version),
                    size_bytes: None,
                    expected_sha256: None,
                }]);
            }
            Ok(Vec::new())
        }
        #[cfg(not(windows))]
        {
            Ok(Vec::new())
        }
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        #[cfg(windows)]
        {
            if !candidate.available.is_known() {
                return Err(Error::Verification {
                    package: candidate.name.clone(),
                    detail: "refusing to install without an exact target version".into(),
                });
            }
            let adapter = nvidia_adapters().await.into_iter().next().ok_or_else(|| {
                Error::unavailable("nvidia-gpu", "no NVIDIA GPU is present any more")
            })?;
            let driver = lookup(&adapter).await?.ok_or_else(|| {
                Error::unavailable(
                    "nvidia-gpu",
                    "NVIDIA no longer offers a driver for this GPU",
                )
            })?;
            if driver.version.trim() != candidate.available.raw().trim() {
                return Err(Error::Verification {
                    package: candidate.name.clone(),
                    detail: format!(
                        "NVIDIA now offers {} instead of the approved {}; scan again",
                        driver.version,
                        candidate.available.raw()
                    ),
                });
            }

            let dir = crate::direct::http::download_dir()?.join(format!(
                "nvidia-{}-{}",
                driver.version,
                chrono::Utc::now().timestamp_millis()
            ));
            tokio::fs::create_dir_all(&dir).await?;
            let file = dir.join(crate::direct::http::file_name_for(
                &driver.url,
                "nvidia-driver.exe",
            ));

            let result = async {
                let client = crate::direct::http::client(None, Duration::from_secs(60 * 60))?;
                tracing::info!(url = %driver.url, "downloading the NVIDIA driver");
                crate::direct::http::download(&client, &driver.url, &file, |_, _| {}).await?;
                let lock = crate::direct::windows_apply::lock_and_verify(
                    &file,
                    None,
                    NVIDIA_SIGNER,
                    &candidate.name,
                )
                .await?;

                let path = file.to_string_lossy().to_string();
                let out = odysync_core::proc::run(
                    &path,
                    &["-s", "-noreboot", "-noeula", "-nofinish", "-nosplash"],
                    INSTALL_TIMEOUT,
                )
                .await;
                drop(lock);
                let out = out?;
                // 0 is success; NVIDIA's setup also uses 1 for "success,
                // reboot needed" in silent mode.
                match out.code {
                    0 | 1 => Ok(()),
                    code => Err(Error::CommandFailed {
                        command: "NVIDIA driver setup".into(),
                        code,
                        stderr: out.stderr.chars().take(500).collect(),
                    }),
                }
            }
            .await;

            let _ = tokio::fs::remove_dir_all(&dir).await;
            result
        }
        #[cfg(not(windows))]
        {
            Err(Error::unavailable(
                "nvidia-gpu",
                format!("{} can only be installed on Windows", candidate.name),
            ))
        }
    }

    async fn installed_version(&self, _candidate: &UpdateCandidate) -> Result<Option<String>> {
        // Read fresh rather than through the memoized adapter list, which
        // still holds the version from before the install.
        #[cfg(windows)]
        {
            let script = "Get-CimInstance Win32_VideoController | \
                          Select-Object Name, PNPDeviceID, DriverVersion | ConvertTo-Json -Compress";
            let out = odysync_core::proc::powershell(script, Duration::from_secs(30)).await?;
            Ok(super::parse_video_controllers(&out.stdout)
                .into_iter()
                .find(|a| a.vendor == GpuVendor::Nvidia)
                .and_then(|a| nvidia_version_from_windows(&a.driver_version)))
        }
        #[cfg(not(windows))]
        {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_driver_versions_convert_to_nvidia_numbering() {
        assert_eq!(
            nvidia_version_from_windows("32.0.16.1692").as_deref(),
            Some("616.92")
        );
        assert_eq!(
            nvidia_version_from_windows("31.0.15.5222").as_deref(),
            Some("552.22")
        );
        assert_eq!(
            nvidia_version_from_windows("32.0.15.7680").as_deref(),
            Some("576.80")
        );
        assert_eq!(nvidia_version_from_windows(""), None);
        assert_eq!(nvidia_version_from_windows("1.2"), None);
        assert_eq!(nvidia_version_from_windows("32.0.x.1692"), None);
    }

    /// The fields NVIDIA's service returned for an RTX 4080, trimmed.
    fn real_answer() -> serde_json::Value {
        serde_json::json!({
            "DriverAttributes": {
                "Name": "GeForce Game Ready Driver",
                "Version": "617.42",
                "IsBeta": "0",
                "IsWHQL": "1",
                "DetailsURL": "https://www.nvidia.com/en-us/drivers/details/280103/",
                "DownloadURL": "https://international-gfe.download.nvidia.com/Windows/617.42/617.42-desktop-win10-win11-64bit-international-dch-whql-g.exe",
                "DownloadURLAdmin": "https://us.download.nvidia.com/Windows/617.42/617.42-desktop-win10-win11-64bit-international-dch-whql.exe"
            }
        })
    }

    #[test]
    fn a_whql_driver_is_read_with_the_standard_package_url() {
        let driver = parse_lookup(&real_answer()).expect("a WHQL driver");
        assert_eq!(driver.version, "617.42");
        assert_eq!(
            driver.url,
            "https://us.download.nvidia.com/Windows/617.42/617.42-desktop-win10-win11-64bit-international-dch-whql.exe"
        );
    }

    #[test]
    fn beta_and_uncertified_drivers_are_not_offered() {
        let mut beta = real_answer();
        beta["DriverAttributes"]["IsBeta"] = "1".into();
        assert!(parse_lookup(&beta).is_none());

        let mut uncertified = real_answer();
        uncertified["DriverAttributes"]["IsWHQL"] = "0".into();
        assert!(parse_lookup(&uncertified).is_none());
    }

    #[test]
    fn a_download_off_nvidias_hosts_is_refused() {
        let mut elsewhere = real_answer();
        elsewhere["DriverAttributes"]["DownloadURLAdmin"] =
            "https://download.nvidia.com.evil.example/driver.exe".into();
        elsewhere["DriverAttributes"]["DownloadURL"] =
            "http://us.download.nvidia.com/Windows/617.42/driver.exe".into();
        assert!(parse_lookup(&elsewhere).is_none());

        assert!(is_nvidia_download(
            "https://us.download.nvidia.com/Windows/1/x.exe"
        ));
        assert!(!is_nvidia_download("https://nvidia.com.example/x.exe"));
        assert!(!is_nvidia_download(
            "https://us.download.nvidia.com/Windows/1/x.zip"
        ));
    }

    #[test]
    fn the_lookup_url_carries_the_device_id_url_encoded() {
        let url = lookup_url("2704", "26100", false);
        assert!(url.starts_with("https://gfwsl.geforce.com/"));
        assert!(url.contains("%222704_10DE%22"));
        assert!(url.contains("%22iLp%22%3A%220%22"));
        assert!(lookup_url("2704", "26100", true).contains("%22iLp%22%3A%221%22"));
        assert!(!url.contains('"') && !url.contains('{'));
    }

    #[test]
    fn backend_kind_is_correct() {
        assert_eq!(NvidiaGpuBackend::new().kind(), BackendKind::NvidiaGpu);
    }
}
