//! Looking for updates while the app sits in the tray.
//!
//! The interval in Settings was saved and shown, and nothing ever looked at
//! it. This is what does: every so often it asks the window to scan again, and
//! the scan command shows a notification when it found something.

use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;

/// How often to check whether a scan is due. Short, so that a changed setting
/// and a computer waking from sleep are noticed soon.
const TICK: Duration = Duration::from_secs(60);

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // The window scans when it opens; the first round starts from there.
        let mut last = Instant::now();
        loop {
            tokio::time::sleep(TICK).await;
            let state = app.state::<AppState>();
            let due = is_due(
                last.elapsed(),
                state.config().scan_interval_hours,
                state.is_applying(),
                odysync_core::platform::power_source().allows_background_work(),
            );
            if due {
                last = Instant::now();
                state.mark_background_scan();
                let _ = app.emit("background-scan", ());
            }
        }
    });
}

/// Whether a background scan should start now. An interval of 0 means the user
/// wants to start scans themselves.
fn is_due(since: Duration, interval_hours: u32, applying: bool, on_mains: bool) -> bool {
    interval_hours > 0
        && since >= Duration::from_secs(u64::from(interval_hours) * 3600)
        && !applying
        && on_mains
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_secs(3600);

    #[test]
    fn a_scan_is_due_after_the_interval_and_not_before() {
        assert!(is_due(6 * HOUR, 6, false, true));
        assert!(!is_due(5 * HOUR, 6, false, true));
    }

    #[test]
    fn manual_only_means_never() {
        assert!(!is_due(1000 * HOUR, 0, false, true));
    }

    #[test]
    fn not_while_installing_and_not_on_battery() {
        assert!(!is_due(6 * HOUR, 6, true, true));
        assert!(!is_due(6 * HOUR, 6, false, false));
    }
}
