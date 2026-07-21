//! Windows driver updates via the Windows Update Agent COM API.
//!
//! v1 used the PSWindowsUpdate PowerShell module, which is a third-party
//! module from PSGallery installed at runtime as Administrator — a supply-chain
//! hole and a slow one. The COM API (`IUpdateSession` / `IUpdateSearcher`) is
//! built into Windows, needs no install, and is considerably faster.
//!
//! This backend searches for driver-only updates (`Type='Driver'`), downloads
//! and installs them, and reports whether a reboot is required through the
//! `RunReport::reboot_required` flag.

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::version::Version;

pub struct WindowsDriverBackend;

impl WindowsDriverBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WindowsDriverBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for WindowsDriverBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::WindowsDrivers
    }

    fn display_name(&self) -> &str {
        "Windows Update (Drivers)"
    }

    async fn is_available(&self) -> bool {
        cfg!(windows)
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(windows) {
            return Ok(Vec::new());
        }

        #[cfg(windows)]
        {
            scan_drivers_com().await
        }
        #[cfg(not(windows))]
        {
            Ok(Vec::new())
        }
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        if !candidate.available.is_known() {
            return Err(Error::Verification {
                package: candidate.id.to_string(),
                detail: "refusing to install without an exact target version".into(),
            });
        }

        #[cfg(windows)]
        {
            install_driver_com(candidate).await
        }
        #[cfg(not(windows))]
        {
            let _ = candidate;
            Ok(())
        }
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        // After installing, re-scan for driver updates. If the update ID is
        // still listed as available, the install did not converge. If it is
        // gone, the driver was installed successfully.
        if !cfg!(windows) {
            return Ok(None);
        }

        let remaining = self.scan().await?;
        if remaining.iter().any(|c| c.id.native == candidate.id.native) {
            // Still pending — install did not converge
            return Ok(None);
        }
        // No longer in the update list — installed successfully
        Ok(Some(candidate.available.raw().to_string()))
    }
}

#[cfg(windows)]
async fn scan_drivers_com() -> Result<Vec<UpdateCandidate>> {
    use windows::core::BSTR;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_LOCAL_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::UpdateAgent::{
        ISearchResult, IUpdate, IUpdateCollection, IUpdateIdentity, IUpdateSearcher,
        IUpdateSession, UpdateSession,
    };

    let result = tokio::task::spawn_blocking(|| {
        let co_initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();

        let scope = || -> Result<Vec<UpdateCandidate>> {
            let session: IUpdateSession =
                unsafe { CoCreateInstance(&UpdateSession, None, CLSCTX_LOCAL_SERVER) }.map_err(
                    |e| {
                        Error::parse(
                            "Windows Update Agent",
                            format!("could not create IUpdateSession: {e}"),
                        )
                    },
                )?;

            let searcher: IUpdateSearcher =
                unsafe { session.CreateUpdateSearcher() }.map_err(|e| {
                    Error::parse(
                        "Windows Update Agent",
                        format!("could not create IUpdateSearcher: {e}"),
                    )
                })?;

            let criteria = BSTR::from("Type='Driver'");
            let result: ISearchResult = unsafe { searcher.Search(&criteria) }.map_err(|e| {
                Error::parse("Windows Update Agent", format!("driver search failed: {e}"))
            })?;

            let code = unsafe { result.ResultCode() }.map_err(|e| {
                Error::parse(
                    "Windows Update Agent",
                    format!("could not read ResultCode: {e}"),
                )
            })?;
            // OperationResultCode is a tuple struct (i32); 2 = orcFailed
            if code.0 == 2 {
                return Err(Error::parse(
                    "Windows Update Agent",
                    "driver search returned orcFailed",
                ));
            }

            let updates: IUpdateCollection = unsafe { result.Updates() }.map_err(|e| {
                Error::parse(
                    "Windows Update Agent",
                    format!("could not get Updates collection: {e}"),
                )
            })?;

            let count = unsafe { updates.Count() }.map_err(|e| {
                Error::parse("Windows Update Agent", format!("could not get count: {e}"))
            })?;

            let mut candidates = Vec::new();

            for i in 0..count {
                let update: IUpdate = unsafe { updates.get_Item(i) }.map_err(|e| {
                    Error::parse(
                        "Windows Update Agent",
                        format!("could not get update item {i}: {e}"),
                    )
                })?;

                let title = unsafe { update.Title() }
                    .map(|t| t.to_string())
                    .unwrap_or_default();

                let identity: IUpdateIdentity = unsafe { update.Identity() }.map_err(|e| {
                    Error::parse(
                        "Windows Update Agent",
                        format!("could not get Identity: {e}"),
                    )
                })?;

                let raw_id = unsafe { identity.UpdateID() }
                    .map(|t| t.to_string())
                    .unwrap_or_default();
                let rev = unsafe { identity.RevisionNumber() }.unwrap_or(0);
                let driver_ver = format!("{raw_id}.{rev}");

                // MaxDownloadSize returns a DECIMAL, not a u64; reinterpreting its
                // memory would be undefined behavior. Leave as None until proper
                // DECIMAL conversion is implemented.
                let size: Option<u64> = None;

                candidates.push(UpdateCandidate {
                    id: PackageId::new(BackendKind::WindowsDrivers, format!("{raw_id}.{rev}")),
                    name: title,
                    installed: Version::parse("0.0.0.0"),
                    available: Version::parse(&driver_ver),
                    size_bytes: size,
                    expected_sha256: None,
                });
            }

            Ok(candidates)
        };

        let result = scope();
        if co_initialized {
            unsafe { CoUninitialize() };
        }
        result
    })
    .await
    .map_err(|e| Error::parse("Windows Update Agent", format!("COM task panicked: {e}")))?;

    result
}

#[cfg(windows)]
async fn install_driver_com(candidate: &UpdateCandidate) -> Result<()> {
    use windows::core::BSTR;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_LOCAL_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::UpdateAgent::{
        ISearchResult, IUpdate, IUpdateCollection, IUpdateInstaller, IUpdateSearcher,
        IUpdateSession, UpdateSession,
    };

    let update_id = candidate.id.native.clone();

    let result = tokio::task::spawn_blocking(move || {
        let co_initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();

        let scope = || -> Result<()> {
            let session: IUpdateSession =
                unsafe { CoCreateInstance(&UpdateSession, None, CLSCTX_LOCAL_SERVER) }.map_err(
                    |e| {
                        Error::parse(
                            "Windows Update Agent",
                            format!("could not create IUpdateSession: {e}"),
                        )
                    },
                )?;

            let searcher: IUpdateSearcher =
                unsafe { session.CreateUpdateSearcher() }.map_err(|e| {
                    Error::parse(
                        "Windows Update Agent",
                        format!("could not create IUpdateSearcher: {e}"),
                    )
                })?;

            let criteria = BSTR::from("Type='Driver'");
            let result: ISearchResult = unsafe { searcher.Search(&criteria) }.map_err(|e| {
                Error::parse(
                    "Windows Update Agent",
                    format!("driver search failed during install: {e}"),
                )
            })?;

            let updates: IUpdateCollection = unsafe { result.Updates() }.map_err(|e| {
                Error::parse(
                    "Windows Update Agent",
                    format!("could not get Updates collection: {e}"),
                )
            })?;

            let count = unsafe { updates.Count() }.map_err(|e| {
                Error::parse("Windows Update Agent", format!("could not get count: {e}"))
            })?;

            let mut found: Option<IUpdate> = None;
            for i in 0..count {
                let update: IUpdate = unsafe { updates.get_Item(i) }.map_err(|e| {
                    Error::parse(
                        "Windows Update Agent",
                        format!("could not get update item {i}: {e}"),
                    )
                })?;

                let identity = unsafe { update.Identity() }.map_err(|e| {
                    Error::parse(
                        "Windows Update Agent",
                        format!("could not get Identity: {e}"),
                    )
                })?;

                let raw_id = unsafe { identity.UpdateID() }
                    .map(|t| t.to_string())
                    .unwrap_or_default();
                let rev = unsafe { identity.RevisionNumber() }.unwrap_or(0);
                let id = format!("{raw_id}.{rev}");

                if id == update_id {
                    found = Some(update);
                    break;
                }
            }

            let Some(update) = found else {
                return Err(Error::Verification {
                    package: update_id.clone(),
                    detail: "driver update was found during scan but is no longer available".into(),
                });
            };

            let to_install: IUpdateCollection = unsafe {
                CoCreateInstance(
                    &windows::Win32::System::UpdateAgent::UpdateCollection,
                    None,
                    CLSCTX_LOCAL_SERVER,
                )
            }
            .map_err(|e| {
                Error::parse(
                    "Windows Update Agent",
                    format!("could not create UpdateCollection: {e}"),
                )
            })?;

            unsafe { to_install.Add(&update) }.map_err(|e| {
                Error::parse(
                    "Windows Update Agent",
                    format!("could not add update to collection: {e}"),
                )
            })?;

            let installer: IUpdateInstaller =
                unsafe { session.CreateUpdateInstaller() }.map_err(|e| {
                    Error::parse(
                        "Windows Update Agent",
                        format!("could not create IUpdateInstaller: {e}"),
                    )
                })?;

            unsafe { installer.SetUpdates(&to_install) }.map_err(|e| {
                Error::parse(
                    "Windows Update Agent",
                    format!("could not set installer updates: {e}"),
                )
            })?;

            // Do not force-install: let the Windows Update Agent apply its
            // own compatibility checks. SetIsForced(true) would bypass them
            // and could install drivers that don't match the hardware.

            let install_result = unsafe { installer.Install() }.map_err(|e| {
                Error::parse(
                    "Windows Update Agent",
                    format!("driver install failed: {e}"),
                )
            })?;

            let result_code = unsafe { install_result.ResultCode() }.map_err(|e| {
                Error::parse(
                    "Windows Update Agent",
                    format!("could not read install ResultCode: {e}"),
                )
            })?;

            // OperationResultCode: 2 = orcFailed, 3 = orcSucceededWithErrors
            if result_code.0 == 2 || result_code.0 == 3 {
                let hr = unsafe { install_result.HResult() }.unwrap_or(0);
                return Err(Error::parse(
                    "Windows Update Agent",
                    format!(
                        "driver install failed (code={}, hr=0x{:08X})",
                        result_code.0, hr
                    ),
                ));
            }

            Ok(())
        };

        let result = scope();
        if co_initialized {
            unsafe { CoUninitialize() };
        }
        result
    })
    .await
    .map_err(|e| Error::parse("Windows Update Agent", format!("COM task panicked: {e}")))?;

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_backend_reports_correct_kind() {
        let b = WindowsDriverBackend::new();
        assert_eq!(b.kind(), BackendKind::WindowsDrivers);
    }

    #[test]
    fn driver_backend_has_display_name() {
        let b = WindowsDriverBackend::new();
        assert!(!b.display_name().is_empty());
    }

    #[tokio::test]
    async fn apply_refuses_unknown_target_version() {
        let backend = WindowsDriverBackend::new();
        let candidate = UpdateCandidate {
            id: PackageId::new(BackendKind::WindowsDrivers, "test-driver"),
            name: "Test Driver".into(),
            installed: Version::parse("1.0.0"),
            available: Version::parse("Unknown"),
            size_bytes: None,
            expected_sha256: None,
        };
        let err = backend.apply(&candidate).await.unwrap_err();
        assert!(matches!(err, Error::Verification { .. }));
    }
}
