//! Firmware update backends for Dell, HP, and Lenovo systems.

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, PackageId, UpdateCandidate};
use odysync_core::version::Version;
use odysync_core::proc;

const SCAN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const INSTALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

// ── Dell Firmware (dcu-cli) ──────────────────────────────────────────────────

pub struct DellFirmwareBackend;

impl DellFirmwareBackend {
    pub fn new() -> Self { Self }
}

impl Default for DellFirmwareBackend {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Backend for DellFirmwareBackend {
    fn kind(&self) -> BackendKind { BackendKind::DellFirmware }
    fn display_name(&self) -> &str { "Dell firmware (dcu-cli)" }

    async fn is_available(&self) -> bool {
        if !cfg!(windows) { return false; }
        let pf = std::env::var("ProgramFiles").unwrap_or_default();
        std::path::Path::new(&pf)
            .join("Dell/CommandUpdate/dcu-cli.exe")
            .exists()
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(windows) { return Ok(Vec::new()); }
        let pf = std::env::var("ProgramFiles").unwrap_or_default();
        let dcu = format!("{pf}/Dell/CommandUpdate/dcu-cli.exe");

        let out = proc::run(&dcu, &["/scan", "/reportFormat:JSON"], SCAN_TIMEOUT).await?;
        // dcu-cli scan output is informational; we report a single candidate
        // if the scan succeeds
        let kind = self.kind();
        if out.success() {
            Ok(vec![UpdateCandidate {
                id: PackageId::new(kind, "dell-firmware"),
                name: "Dell System Firmware".to_string(),
                installed: Version::Unknown(String::new()),
                available: Version::Unknown(String::new()),
                size_bytes: None,
                expected_sha256: None,
            }])
        } else {
            Ok(Vec::new())
        }
    }

    async fn apply(&self, _candidate: &UpdateCandidate) -> Result<()> {
        if !cfg!(windows) {
            return Err(Error::Verification {
                package: _candidate.id.to_string(),
                detail: "Dell firmware updates are Windows-only".into(),
            });
        }
        let pf = std::env::var("ProgramFiles").unwrap_or_default();
        let dcu = format!("{pf}/Dell/CommandUpdate/dcu-cli.exe");
        let out = proc::run(&dcu, &["/applyUpdates", "/silent", "/reboot=disable"], INSTALL_TIMEOUT).await?;
        if !out.success() {
            return Err(Error::CommandFailed {
                command: "dcu-cli /applyUpdates".into(),
                code: out.code,
                stderr: out.stderr,
            });
        }
        Ok(())
    }

    async fn installed_version(&self, _candidate: &UpdateCandidate) -> Result<Option<String>> {
        Ok(None)
    }
}

// ── HP Firmware (HPIA) ───────────────────────────────────────────────────────

pub struct HpFirmwareBackend;

impl HpFirmwareBackend {
    pub fn new() -> Self { Self }
}

impl Default for HpFirmwareBackend {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Backend for HpFirmwareBackend {
    fn kind(&self) -> BackendKind { BackendKind::HpFirmware }
    fn display_name(&self) -> &str { "HP firmware (HPIA)" }

    async fn is_available(&self) -> bool {
        if !cfg!(windows) { return false; }
        let pf = std::env::var("ProgramFiles").unwrap_or_default();
        std::path::Path::new(&pf)
            .join("HP/HP Image Assistant/HPIA.exe")
            .exists()
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(windows) { return Ok(Vec::new()); }
        let pf = std::env::var("ProgramFiles").unwrap_or_default();
        let hpia = format!("{pf}/HP/HP Image Assistant/HPIA.exe");

        let out = proc::run(&hpia, &["/Analyze", "/Action:Analyze", "/Silent", "/ReportFolder:%TEMP%"], SCAN_TIMEOUT).await?;
        let kind = self.kind();
        if out.success() {
            Ok(vec![UpdateCandidate {
                id: PackageId::new(kind, "hp-firmware"),
                name: "HP System Firmware".to_string(),
                installed: Version::Unknown(String::new()),
                available: Version::Unknown(String::new()),
                size_bytes: None,
                expected_sha256: None,
            }])
        } else {
            Ok(Vec::new())
        }
    }

    async fn apply(&self, _candidate: &UpdateCandidate) -> Result<()> {
        if !cfg!(windows) {
            return Err(Error::Verification {
                package: _candidate.id.to_string(),
                detail: "HP firmware updates are Windows-only".into(),
            });
        }
        let pf = std::env::var("ProgramFiles").unwrap_or_default();
        let hpia = format!("{pf}/HP/HP Image Assistant/HPIA.exe");
        let out = proc::run(&hpia, &["/Apply", "/Action:Install", "/Silent", "/ReportFolder:%TEMP%"], INSTALL_TIMEOUT).await?;
        if !out.success() {
            return Err(Error::CommandFailed {
                command: "HPIA /Apply".into(),
                code: out.code,
                stderr: out.stderr,
            });
        }
        Ok(())
    }

    async fn installed_version(&self, _candidate: &UpdateCandidate) -> Result<Option<String>> {
        Ok(None)
    }
}

// ── Lenovo Firmware (SUHelper) ───────────────────────────────────────────────

pub struct LenovoFirmwareBackend;

impl LenovoFirmwareBackend {
    pub fn new() -> Self { Self }
}

impl Default for LenovoFirmwareBackend {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Backend for LenovoFirmwareBackend {
    fn kind(&self) -> BackendKind { BackendKind::LenovoFirmware }
    fn display_name(&self) -> &str { "Lenovo firmware (SUHelper)" }

    async fn is_available(&self) -> bool {
        if !cfg!(windows) { return false; }
        let pf = std::env::var("ProgramFiles").unwrap_or_default();
        std::path::Path::new(&pf)
            .join("Lenovo/System Update/suhelper.exe")
            .exists()
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        if !cfg!(windows) { return Ok(Vec::new()); }
        let pf = std::env::var("ProgramFiles").unwrap_or_default();
        let su = format!("{pf}/Lenovo/System Update/tvsu.exe");

        let out = proc::run(&su, &["/CM", "/search", "/action", "List", "/silent"], SCAN_TIMEOUT).await?;
        let kind = self.kind();
        if out.success() {
            Ok(vec![UpdateCandidate {
                id: PackageId::new(kind, "lenovo-firmware"),
                name: "Lenovo System Firmware".to_string(),
                installed: Version::Unknown(String::new()),
                available: Version::Unknown(String::new()),
                size_bytes: None,
                expected_sha256: None,
            }])
        } else {
            Ok(Vec::new())
        }
    }

    async fn apply(&self, _candidate: &UpdateCandidate) -> Result<()> {
        if !cfg!(windows) {
            return Err(Error::Verification {
                package: _candidate.id.to_string(),
                detail: "Lenovo firmware updates are Windows-only".into(),
            });
        }
        let pf = std::env::var("ProgramFiles").unwrap_or_default();
        let su = format!("{pf}/Lenovo/System Update/tvsu.exe");
        let out = proc::run(&su, &["/CM", "/install", "/action", "Install", "/silent", "/noreboot"], INSTALL_TIMEOUT).await?;
        if !out.success() {
            return Err(Error::CommandFailed {
                command: "tvsu /install".into(),
                code: out.code,
                stderr: out.stderr,
            });
        }
        Ok(())
    }

    async fn installed_version(&self, _candidate: &UpdateCandidate) -> Result<Option<String>> {
        Ok(None)
    }
}
