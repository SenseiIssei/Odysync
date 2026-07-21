//! Cross-platform scheduled task management.
//!
//! Windows: Task Scheduler via `schtasks`.
//! macOS: launchd via a plist in `~/Library/LaunchAgents`.
//! Linux: systemd user timer via `~/.config/systemd/user/`.
//!
//! The schedule is a simple daily or weekly trigger at a specified time.
//! The scheduled command is `odysync apply --yes` (or the user's custom args).

use serde::{Deserialize, Serialize};

use odysync_core::error::{Error, Result};
use odysync_core::proc;

/// The task name used across all platforms.
pub const DEFAULT_TASK_NAME: &str = "Odysync";

/// How often to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleFrequency {
    Daily,
    Weekly,
}

impl ScheduleFrequency {
    fn id(&self) -> &'static str {
        match self {
            ScheduleFrequency::Daily => "daily",
            ScheduleFrequency::Weekly => "weekly",
        }
    }
}

impl std::fmt::Display for ScheduleFrequency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}

/// A schedule specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleSpec {
    pub frequency: ScheduleFrequency,
    /// 24-hour time, e.g. "09:00".
    pub time: String,
    /// Task name for later removal.
    pub task_name: String,
    /// Extra arguments to pass to `odysync`.
    pub extra_args: Vec<String>,
}

/// Resolve the path to the current executable, for the scheduler to launch.
fn current_exe() -> Result<String> {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| Error::Config(format!("could not resolve current executable: {e}")))
}

// ── Windows: Task Scheduler ─────────────────────────────────────────────────

#[cfg(windows)]
pub async fn create_schedule(spec: &ScheduleSpec) -> Result<()> {
    let exe = current_exe()?;
    let mut args = vec!["apply".to_string(), "--yes".to_string()];
    args.extend(spec.extra_args.iter().cloned());
    let tr = format!("\"{exe}\" {}", args.join(" "));

    let sc = match spec.frequency {
        ScheduleFrequency::Daily => "DAILY",
        ScheduleFrequency::Weekly => "WEEKLY",
    };

    let out = proc::run(
        "schtasks",
        &[
            "/Create",
            "/SC",
            sc,
            "/TN",
            &spec.task_name,
            "/TR",
            &tr,
            "/ST",
            &spec.time,
            "/F",
        ],
        std::time::Duration::from_secs(30),
    )
    .await?;

    if out.success() {
        Ok(())
    } else {
        Err(Error::CommandFailed {
            command: "schtasks /Create".into(),
            code: out.code,
            stderr: out.stderr,
        })
    }
}

#[cfg(windows)]
pub async fn remove_schedule(task_name: &str) -> Result<()> {
    let out = proc::run(
        "schtasks",
        &["/Delete", "/TN", task_name, "/F"],
        std::time::Duration::from_secs(15),
    )
    .await?;

    if out.success() {
        Ok(())
    } else {
        Err(Error::CommandFailed {
            command: "schtasks /Delete".into(),
            code: out.code,
            stderr: out.stderr,
        })
    }
}

#[cfg(windows)]
pub async fn schedule_exists(task_name: &str) -> bool {
    matches!(
        proc::run(
            "schtasks",
            &["/Query", "/TN", task_name],
            std::time::Duration::from_secs(15),
        )
        .await,
        Ok(o) if o.success()
    )
}

// ── macOS: launchd ──────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn plist_path(task_name: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("dev.odysync.{task_name}.plist"))
}

#[cfg(target_os = "macos")]
pub async fn create_schedule(spec: &ScheduleSpec) -> Result<()> {
    let exe = current_exe()?;
    let mut args = vec!["apply".to_string(), "--yes".to_string()];
    args.extend(spec.extra_args.iter().cloned());

    let label = format!("dev.odysync.{}", spec.task_name);
    let calendar = match spec.frequency {
        ScheduleFrequency::Daily => {
            // Run every day at the specified time.
            let (h, m) = parse_time(&spec.time)?;
            format!(
                "<dict><key>Hour</key><integer>{h}</integer><key>Minute</key><integer>{m}</integer></dict>"
            )
        }
        ScheduleFrequency::Weekly => {
            let (h, m) = parse_time(&spec.time)?;
            format!(
                "<dict><key>Hour</key><integer>{h}</integer><key>Minute</key><integer>{m}</integer><key>Weekday</key><integer>1</integer></dict>"
            )
        }
    };

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    {args_xml}
  </array>
  <key>StartCalendarInterval</key>
  {calendar}
  <key>RunAtLoad</key><false/>
</dict>
</plist>"#,
        args_xml = args
            .iter()
            .map(|a| format!("<string>{a}</string>"))
            .collect::<Vec<_>>()
            .join("\n    "),
    );

    let path = plist_path(&spec.task_name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &plist)?;

    let out = proc::run(
        "launchctl",
        &["load", &path.to_string_lossy()],
        std::time::Duration::from_secs(15),
    )
    .await?;

    if out.success() {
        Ok(())
    } else {
        Err(Error::CommandFailed {
            command: "launchctl load".into(),
            code: out.code,
            stderr: out.stderr,
        })
    }
}

#[cfg(target_os = "macos")]
pub async fn remove_schedule(task_name: &str) -> bool {
    let path = plist_path(task_name);
    let _ = proc::run(
        "launchctl",
        &["unload", &path.to_string_lossy()],
        std::time::Duration::from_secs(15),
    )
    .await;
    let _ = std::fs::remove_file(&path);
    true
}

#[cfg(target_os = "macos")]
pub async fn schedule_exists(task_name: &str) -> bool {
    plist_path(task_name).exists()
}

// ── Linux: systemd user timer ───────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn unit_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| {
        Error::Config("HOME is not set; cannot determine systemd user directory".into())
    })?;
    Ok(PathBuf::from(home).join(".config/systemd/user"))
}

#[cfg(target_os = "linux")]
pub async fn create_schedule(spec: &ScheduleSpec) -> Result<()> {
    let exe = current_exe()?;
    let mut args = vec!["apply".to_string(), "--yes".to_string()];
    args.extend(spec.extra_args.iter().cloned());

    let dir = unit_dir()?;
    std::fs::create_dir_all(&dir)?;

    let unit = &spec.task_name;
    let service_name = format!("dev.odysync.{unit}.service");
    let timer_name = format!("dev.odysync.{unit}.timer");

    let exec_start = format!(
        "{exe} {}",
        args.join(" ")
    );

    let service = format!(
        "[Unit]\nDescription=Odysync scheduled run\n\n[Service]\nType=oneshot\nExecStart={exec_start}\n"
    );

    let on_calendar = match spec.frequency {
        ScheduleFrequency::Daily => format!("*-*-* {}:00", spec.time),
        ScheduleFrequency::Weekly => format!("Mon *-*-* {}:00", spec.time),
    };

    let timer = format!(
        "[Unit]\nDescription=Run Odysync {freq}\n\n[Timer]\nOnCalendar={on_calendar}\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n",
        freq = spec.frequency,
    );

    std::fs::write(dir.join(&service_name), &service)?;
    std::fs::write(dir.join(&timer_name), &timer)?;

    let out = proc::run(
        "systemctl",
        &["--user", "daemon-reload"],
        std::time::Duration::from_secs(15),
    )
    .await?;
    if !out.success() {
        return Err(Error::CommandFailed {
            command: "systemctl --user daemon-reload".into(),
            code: out.code,
            stderr: out.stderr,
        });
    }

    let out = proc::run(
        "systemctl",
        &["--user", "enable", &timer_name],
        std::time::Duration::from_secs(15),
    )
    .await?;
    if !out.success() {
        return Err(Error::CommandFailed {
            command: format!("systemctl --user enable {timer_name}"),
            code: out.code,
            stderr: out.stderr,
        });
    }

    let out = proc::run(
        "systemctl",
        &["--user", "start", &timer_name],
        std::time::Duration::from_secs(15),
    )
    .await?;
    if !out.success() {
        return Err(Error::CommandFailed {
            command: format!("systemctl --user start {timer_name}"),
            code: out.code,
            stderr: out.stderr,
        });
    }

    Ok(())
}

#[cfg(target_os = "linux")]
pub async fn remove_schedule(task_name: &str) -> bool {
    let dir = match unit_dir() {
        Ok(d) => d,
        Err(_) => return false,
    };

    let timer_name = format!("dev.odysync.{task_name}.timer");
    let service_name = format!("dev.odysync.{task_name}.service");

    let _ = proc::run(
        "systemctl",
        &["--user", "disable", &timer_name],
        std::time::Duration::from_secs(15),
    )
    .await;
    let _ = proc::run(
        "systemctl",
        &["--user", "stop", &timer_name],
        std::time::Duration::from_secs(15),
    )
    .await;

    let _ = std::fs::remove_file(dir.join(&timer_name));
    let _ = std::fs::remove_file(dir.join(&service_name));

    let _ = proc::run(
        "systemctl",
        &["--user", "daemon-reload"],
        std::time::Duration::from_secs(15),
    )
    .await;

    true
}

#[cfg(target_os = "linux")]
pub async fn schedule_exists(task_name: &str) -> bool {
    let dir = match unit_dir() {
        Ok(d) => d,
        Err(_) => return false,
    };
    dir.join(format!("dev.odysync.{task_name}.timer")).exists()
}

// ── Non-Windows/macOS/Linux fallback ────────────────────────────────────────

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub async fn create_schedule(_spec: &ScheduleSpec) -> Result<()> {
    Err(Error::Config("scheduling is not supported on this platform".into()))
}
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub async fn remove_schedule(_task_name: &str) -> bool {
    false
}
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub async fn schedule_exists(_task_name: &str) -> bool {
    false
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Parse "HH:MM" into (hour, minute).
#[allow(dead_code)]
fn parse_time(time: &str) -> Result<(i32, i32)> {
    let (h, m) = time
        .split_once(':')
        .ok_or_else(|| Error::Config(format!("invalid time format: '{time}', expected HH:MM")))?;
    let h: i32 = h
        .parse()
        .map_err(|_| Error::Config(format!("invalid hour in '{time}'")))?;
    let m: i32 = m
        .parse()
        .map_err(|_| Error::Config(format!("invalid minute in '{time}'")))?;
    if !(0..=23).contains(&h) || !(0..=59).contains(&m) {
        return Err(Error::Config(format!("time '{time}' is out of range")));
    }
    Ok((h, m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_time_accepts_24h_format() {
        assert_eq!(parse_time("09:00").unwrap(), (9, 0));
        assert_eq!(parse_time("23:59").unwrap(), (23, 59));
        assert_eq!(parse_time("00:00").unwrap(), (0, 0));
    }

    #[test]
    fn parse_time_rejects_invalid_input() {
        assert!(parse_time("9am").is_err());
        assert!(parse_time("25:00").is_err());
        assert!(parse_time("12:60").is_err());
        assert!(parse_time("").is_err());
    }

    #[test]
    fn schedule_frequency_serializes_as_lowercase() {
        let json = serde_json::to_string(&ScheduleFrequency::Daily).unwrap();
        assert_eq!(json, "\"daily\"");
        let json = serde_json::to_string(&ScheduleFrequency::Weekly).unwrap();
        assert_eq!(json, "\"weekly\"");
    }
}
