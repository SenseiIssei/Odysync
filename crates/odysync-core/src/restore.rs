//! System Restore point creation (Windows).
//!
//! On Windows, a restore point is created once per run, before the first
//! `apply`. When System Protection is off or the process is not elevated, the
//! guard skips gracefully — the caller is informed but the run is not aborted.
//!
//! On non-Windows platforms this is a no-op.

use crate::error::Result;

/// RAII guard that creates a restore point on construction and is a no-op on
/// drop. Creating once per run — not per package — is the design choice: a
/// restore point captures the system state before *the batch*, and individual
/// package rollbacks are not supported (see the roadmap's "Known gaps" section).
pub struct RestorePointGuard {
    created: bool,
}

impl RestorePointGuard {
    /// Attempt to create a restore point named `description`.
    ///
    /// Returns a guard whose `created()` tells the caller whether it actually
    /// landed. A `false` result is not an error — System Protection may be off.
    #[cfg(windows)]
    pub async fn new(description: &str) -> Result<Self> {
        let created = create_restore_point(description).await;
        Ok(Self { created })
    }

    #[cfg(not(windows))]
    pub async fn new(_description: &str) -> Result<Self> {
        Ok(Self { created: false })
    }

    pub fn created(&self) -> bool {
        self.created
    }
}

#[cfg(windows)]
async fn create_restore_point(description: &str) -> bool {
    use windows::Win32::System::Restore::{
        SRSetRestorePointA, BEGIN_SYSTEM_CHANGE, MODIFY_SETTINGS, RESTOREPOINTINFOA, STATEMGRSTATUS,
    };

    // System Restore requires elevation. If we are not elevated, skip silently.
    if !crate::platform::is_elevated() {
        tracing::info!("restore point skipped: process is not elevated");
        return false;
    }

    // SAFETY: SRSetRestorePointA is a well-defined Win32 API. The structs are
    // zeroed and populated with valid values.
    let mut desc_buf = [0i8; 64];
    let bytes = description.as_bytes();
    let len = bytes.len().min(63);
    for (i, &b) in bytes[..len].iter().enumerate() {
        desc_buf[i] = b as i8;
    }

    let info = RESTOREPOINTINFOA {
        dwEventType: BEGIN_SYSTEM_CHANGE,
        dwRestorePtType: MODIFY_SETTINGS,
        llSequenceNumber: 0,
        szDescription: desc_buf,
    };
    let mut status = STATEMGRSTATUS::default();

    let ok = unsafe { SRSetRestorePointA(&info, &mut status) };

    if ok.as_bool() {
        let seq = status.llSequenceNumber;
        tracing::info!(seq, "restore point created");
        true
    } else {
        tracing::info!(
            "restore point not created (System Protection may be disabled or a recent point exists)"
        );
        false
    }
}

#[cfg(not(windows))]
async fn create_restore_point(_description: &str) -> bool {
    false
}
