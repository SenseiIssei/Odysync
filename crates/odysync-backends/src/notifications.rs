//! Desktop notifications for update events.
//!
//! Fires native OS notifications when updates are found, applied, or fail.
//! On Windows this uses `powershell.exe` with `BurntToast` or the built-in
//! toast XML API.  On macOS it uses `osascript`.  On Linux it uses
//! `notify-send`.

use odysync_core::model::ApplyOutcome;

/// Event types that trigger a notification.
#[derive(Debug, Clone)]
pub enum NotificationEvent {
    /// Updates were found during a scan.
    UpdatesFound { count: usize },
    /// An update was successfully applied.
    UpdateSucceeded { name: String, version: String },
    /// An update failed to apply.
    UpdateFailed { name: String, detail: String },
    /// All updates in a batch completed.
    BatchComplete { updated: usize, failed: usize },
}

impl NotificationEvent {
    fn title(&self) -> &str {
        match self {
            Self::UpdatesFound { .. } => "Updates Available",
            Self::UpdateSucceeded { .. } => "Update Installed",
            Self::UpdateFailed { .. } => "Update Failed",
            Self::BatchComplete { .. } => "Updates Complete",
        }
    }

    fn body(&self) -> String {
        match self {
            Self::UpdatesFound { count } => {
                if *count == 1 {
                    "1 update is available.".into()
                } else {
                    format!("{count} updates are available.")
                }
            }
            Self::UpdateSucceeded { name, version } => {
                format!("{name} updated to {version}.")
            }
            Self::UpdateFailed { name, detail } => {
                format!("{name}: {detail}")
            }
            Self::BatchComplete { updated, failed } => {
                if *failed == 0 {
                    format!("{updated} update(s) installed successfully.")
                } else {
                    format!("{updated} succeeded, {failed} failed.")
                }
            }
        }
    }
}

/// Fire a desktop notification for the given event.
pub async fn notify(event: &NotificationEvent) {
    let title = event.title();
    let body = event.body();

    #[cfg(windows)]
    {
        notify_windows(title, &body).await;
    }

    #[cfg(target_os = "macos")]
    {
        notify_macos(title, &body).await;
    }

    #[cfg(target_os = "linux")]
    {
        notify_linux(title, &body).await;
    }

    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = (title, body);
    }
}

/// Create a notification event from an apply outcome.
pub fn event_from_outcome(name: &str, outcome: &ApplyOutcome) -> Option<NotificationEvent> {
    match outcome {
        ApplyOutcome::Updated { to, .. } => Some(NotificationEvent::UpdateSucceeded {
            name: name.to_string(),
            version: to.clone(),
        }),
        ApplyOutcome::Failed { detail } => Some(NotificationEvent::UpdateFailed {
            name: name.to_string(),
            detail: detail.clone(),
        }),
        ApplyOutcome::DidNotConverge { expected, actual } => {
            Some(NotificationEvent::UpdateFailed {
                name: name.to_string(),
                detail: format!("expected {expected}, got {actual}"),
            })
        }
        ApplyOutcome::VerificationFailed { detail } => Some(NotificationEvent::UpdateFailed {
            name: name.to_string(),
            detail: detail.clone(),
        }),
        ApplyOutcome::Skipped { .. } => None,
    }
}

#[cfg(windows)]
async fn notify_windows(title: &str, body: &str) {
    use std::process::Command;

    // Try BurntToast first (if installed), then fall back to raw toast XML.
    let script = format!(
        r#"
        try {{
            Import-Module BurntToast -ErrorAction Stop
            New-BurntToastNotification -Text '{}', '{}'
        }} catch {{
            [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
            $template = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02)
            $text = $template.GetElementsByTagName('text')
            $text.Item(0).AppendChild($template.CreateTextNode('{}')) | Out-Null
            $text.Item(1).AppendChild($template.CreateTextNode('{}')) | Out-Null
            $notifier = [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('Odysync')
            $notifier.Show([Windows.UI.Notifications.ToastNotification]::new($template))
        }}
        "#,
        title.replace('\'', "''"),
        body.replace('\'', "''"),
        title.replace('\'', "''"),
        body.replace('\'', "''"),
    );

    let _ = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .spawn();
}

#[cfg(target_os = "macos")]
async fn notify_macos(title: &str, body: &str) {
    use std::process::Command;

    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        body.replace('"', "\\\""),
        title.replace('"', "\\\"")
    );

    let _ = Command::new("osascript").args(["-e", &script]).spawn();
}

#[cfg(target_os = "linux")]
async fn notify_linux(title: &str, body: &str) {
    use std::process::Command;

    let _ = Command::new("notify-send")
        .args(["--app-name=Odysync", title, body])
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;
    use odysync_core::model::SkipReason;

    #[test]
    fn updates_found_title_and_body() {
        let event = NotificationEvent::UpdatesFound { count: 3 };
        assert_eq!(event.title(), "Updates Available");
        assert_eq!(event.body(), "3 updates are available.");
    }

    #[test]
    fn updates_found_singular() {
        let event = NotificationEvent::UpdatesFound { count: 1 };
        assert_eq!(event.body(), "1 update is available.");
    }

    #[test]
    fn update_succeeded_title_and_body() {
        let event = NotificationEvent::UpdateSucceeded {
            name: "NVIDIA Driver".into(),
            version: "537.42".into(),
        };
        assert_eq!(event.title(), "Update Installed");
        assert_eq!(event.body(), "NVIDIA Driver updated to 537.42.");
    }

    #[test]
    fn update_failed_title_and_body() {
        let event = NotificationEvent::UpdateFailed {
            name: "AMD Driver".into(),
            detail: "network error".into(),
        };
        assert_eq!(event.title(), "Update Failed");
        assert_eq!(event.body(), "AMD Driver: network error");
    }

    #[test]
    fn batch_complete_all_success() {
        let event = NotificationEvent::BatchComplete {
            updated: 5,
            failed: 0,
        };
        assert_eq!(event.title(), "Updates Complete");
        assert_eq!(event.body(), "5 update(s) installed successfully.");
    }

    #[test]
    fn batch_complete_with_failures() {
        let event = NotificationEvent::BatchComplete {
            updated: 3,
            failed: 2,
        };
        assert_eq!(event.body(), "3 succeeded, 2 failed.");
    }

    #[test]
    fn event_from_updated_outcome() {
        let outcome = ApplyOutcome::Updated {
            from: "1.0".into(),
            to: "2.0".into(),
        };
        let event = event_from_outcome("Test", &outcome);
        assert!(matches!(
            event,
            Some(NotificationEvent::UpdateSucceeded { .. })
        ));
    }

    #[test]
    fn event_from_failed_outcome() {
        let outcome = ApplyOutcome::Failed {
            detail: "broken".into(),
        };
        let event = event_from_outcome("Test", &outcome);
        assert!(matches!(
            event,
            Some(NotificationEvent::UpdateFailed { .. })
        ));
    }

    #[test]
    fn event_from_skipped_outcome_is_none() {
        let outcome = ApplyOutcome::Skipped {
            reason: SkipReason::Excluded,
        };
        let event = event_from_outcome("Test", &outcome);
        assert!(event.is_none());
    }
}
