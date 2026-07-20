//! Maintenance actions that are not package updates.
//!
//! Temp cleanup, Recycle Bin, DISM/SFC health checks and the startup-programs
//! viewer are system-level housekeeping tasks. They do not flow through the
//! update policy because they are not versioned package operations — there is
//! nothing to compare, pin or hold. Keeping them behind a separate trait makes
//! that boundary explicit and prevents a backend from accidentally routing
//! them through the safety engine.

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Which maintenance action to perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MaintenanceKind {
    /// Delete contents of TEMP and system temp directories.
    TempCleanup,
    /// Empty the Recycle Bin (Windows) or trash (macOS/Linux).
    CleanRecycleBin,
    /// Run DISM ScanHealth + RestoreHealth and SFC /scannow (Windows only).
    SystemHealth,
    /// List programs registered to run at startup.
    StartupPrograms,
}

impl MaintenanceKind {
    pub fn id(&self) -> &'static str {
        match self {
            MaintenanceKind::TempCleanup => "temp-cleanup",
            MaintenanceKind::CleanRecycleBin => "clean-recycle-bin",
            MaintenanceKind::SystemHealth => "system-health",
            MaintenanceKind::StartupPrograms => "startup-programs",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            MaintenanceKind::TempCleanup => "Temp folder cleanup",
            MaintenanceKind::CleanRecycleBin => "Empty Recycle Bin",
            MaintenanceKind::SystemHealth => "System health (DISM + SFC)",
            MaintenanceKind::StartupPrograms => "Startup programs",
        }
    }
}

impl std::fmt::Display for MaintenanceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// The outcome of a single maintenance action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaintenanceResult {
    pub kind: MaintenanceKind,
    pub success: bool,
    /// Human-readable summary, e.g. "Removed 42 temp items".
    pub summary: String,
}

/// A system-level maintenance action, separate from the [`Backend`] trait.
///
/// Implementations live in `sensei-backends` and are platform-specific. They
/// must be idempotent and safe to run without user interaction — a failure is
/// reported, not retried.
#[async_trait::async_trait]
pub trait Maintenance: Send + Sync {
    fn kind(&self) -> MaintenanceKind;

    /// Whether this action can run on the current machine.
    async fn is_available(&self) -> bool;

    /// Perform the action.
    async fn run(&self) -> Result<MaintenanceResult>;
}
