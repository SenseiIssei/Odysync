//! Cross-platform scheduled task management.
//!
//! Windows: Task Scheduler via `schtasks`.
//! macOS: launchd via a plist in `~/Library/LaunchAgents`.
//! Linux: systemd user timer via `~/.config/systemd/user/`.
//!
//! The schedule is a simple daily or weekly trigger at a specified time.
//! The scheduled command is `odysync apply --yes --unattended`, or
//! `odysync care --yes` for automatic care (updates plus the safe cleanup),
//! followed by any extra arguments. Both leave drivers, firmware, system
//! upgrades and developer packages to a person.

// Only the launchd (macOS) and systemd (Linux) paths build file paths; on
// Windows this would be an unused import, which `-D warnings` rejects.
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::path::PathBuf;

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

/// What a schedule runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScheduledCommand {
    /// Install the updates that pass every safety rule.
    #[default]
    Apply,
    /// The updates, then the cleanup categories that are safe by default.
    Care,
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
    /// The `odysync` command-line binary to run.
    ///
    /// `None` means "this executable", which is right for the CLI. The GUI
    /// must say where the CLI is: scheduling its own executable would open a
    /// window at the scheduled time and update nothing, because the GUI does
    /// not take command-line verbs.
    #[serde(default)]
    pub program: Option<String>,
    /// What to run. Older saved specs have no field and meant `apply`.
    #[serde(default)]
    pub command: ScheduledCommand,
}

impl ScheduleSpec {
    /// The arguments the scheduler passes to [`Self::program`].
    pub fn args(&self) -> Vec<String> {
        let verb = match self.command {
            ScheduledCommand::Apply => "apply",
            ScheduledCommand::Care => "care",
        };
        let mut args = vec![verb.to_string(), "--yes".to_string()];
        if self.command == ScheduledCommand::Apply {
            args.push("--unattended".to_string());
        }
        args.extend(self.extra_args.iter().cloned());
        args
    }

    fn program(&self) -> Result<String> {
        match &self.program {
            Some(p) => Ok(p.clone()),
            None => current_exe(),
        }
    }
}

/// Refuse a task name that is more than a name.
///
/// It becomes a file name on macOS and Linux and a path in the Task Scheduler,
/// where `..` and separators would reach other people's schedules.
fn check_task_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ' '));
    if ok {
        Ok(())
    } else {
        Err(Error::Config(format!(
            "{name:?} is not a usable schedule name; use letters, digits, spaces, '-' and '_'"
        )))
    }
}

/// `text` inside an XML element.
#[cfg(any(windows, target_os = "macos", test))]
fn xml_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The Task Scheduler definition for `spec`.
///
/// `schtasks /SC DAILY` cannot say any of what matters here, and its defaults
/// are wrong for an updater: a run missed because the computer was off is
/// dropped for good, and a run in progress is **stopped** when a laptop is
/// unplugged, in the middle of whatever installer was running.
#[cfg(any(windows, test))]
fn task_xml(spec: &ScheduleSpec, program: &str, now: jiff::civil::DateTime) -> Result<String> {
    // The next time this hour comes round. A start in the past would count as
    // missed, and `StartWhenAvailable` would make up for it right away: whoever
    // chose nine in the morning at three in the afternoon would see it install.
    let (h, m) = parse_time(&spec.time)?;
    let start = next_start(now, h, m)?;
    let schedule = match spec.frequency {
        ScheduleFrequency::Daily => {
            "<ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay>".to_string()
        }
        ScheduleFrequency::Weekly => "<ScheduleByWeek><WeeksInterval>1</WeeksInterval>\
             <DaysOfWeek><Monday /></DaysOfWeek></ScheduleByWeek>"
            .to_string(),
    };
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>Odysync scheduled run</Description></RegistrationInfo>
  <Triggers>
    <CalendarTrigger>
      <StartBoundary>{start}</StartBoundary>
      {schedule}
    </CalendarTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>true</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>false</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>true</RunOnlyIfNetworkAvailable>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <Arguments>{arguments}</Arguments>
    </Exec>
  </Actions>
</Task>"#,
        command = xml_text(program),
        arguments = xml_text(
            &spec
                .args()
                .iter()
                .map(|a| windows_quoted(a))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        start = start.strftime("%Y-%m-%dT%H:%M:%S"),
    ))
}

/// The next `h:m` after `now`, in the same local time.
#[cfg(any(windows, test))]
fn next_start(now: jiff::civil::DateTime, h: i32, m: i32) -> Result<jiff::civil::DateTime> {
    use jiff::ToSpan;
    let bad = |e: jiff::Error| Error::Config(format!("could not work out the first run: {e}"));
    let time = jiff::civil::Time::new(h as i8, m as i8, 0, 0).map_err(bad)?;
    let today = now.date().to_datetime(time);
    if today > now {
        Ok(today)
    } else {
        today.checked_add(1.day()).map_err(bad)
    }
}

/// Local time as Windows shows it.
#[cfg(windows)]
fn local_now() -> Result<jiff::civil::DateTime> {
    // SAFETY: no arguments, returns a plain structure.
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    jiff::civil::DateTime::new(
        t.wYear as i16,
        t.wMonth as i8,
        t.wDay as i8,
        t.wHour as i8,
        t.wMinute as i8,
        t.wSecond as i8,
        0,
    )
    .map_err(|e| Error::Config(format!("the clock shows no valid time: {e}")))
}

/// One argument of a Windows command line, quoted if it has to be.
#[cfg(any(windows, test))]
fn windows_quoted(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    // Backslashes count double only in front of a quote.
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes + 1));
                backslashes = 0;
            }
            _ => backslashes = 0,
        }
        out.push(c);
    }
    out.push_str(&"\\".repeat(backslashes));
    out.push('"');
    out
}

/// One argument of a systemd command line.
#[cfg(any(target_os = "linux", test))]
fn systemd_quoted(arg: &str) -> String {
    let escaped = arg
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('$', "$$");
    format!("\"{escaped}\"")
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
    check_task_name(&spec.task_name)?;
    let xml = task_xml(spec, &spec.program()?, local_now()?)?;

    // schtasks reads the definition from a file, in the UTF-16 it declares.
    let dir = crate::direct::http::DownloadDir::new("schedule")?;
    let file = dir.path().join("task.xml");
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(&file, bytes)?;

    let out = proc::run(
        "schtasks",
        &[
            "/Create",
            "/TN",
            &spec.task_name,
            "/XML",
            &file.to_string_lossy(),
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
    check_task_name(task_name)?;
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
    check_task_name(&spec.task_name)?;
    let exe = xml_text(&spec.program()?);
    let args = spec.args();

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
            .map(|a| format!("<string>{}</string>", xml_text(a)))
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

/// Returns `Result<()>` to match the Windows implementation. These signatures
/// used to disagree — `Result` on Windows, `bool` everywhere else — so callers
/// written against one platform could not compile on the others.
#[cfg(target_os = "macos")]
pub async fn remove_schedule(task_name: &str) -> Result<()> {
    check_task_name(task_name)?;
    let path = plist_path(task_name);
    let _ = proc::run(
        "launchctl",
        &["unload", &path.to_string_lossy()],
        std::time::Duration::from_secs(15),
    )
    .await;
    // An absent plist means there was nothing scheduled, which is the desired
    // end state rather than a failure.
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
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
    check_task_name(&spec.task_name)?;
    parse_time(&spec.time)?;
    let exe = spec.program()?;
    let args = spec.args();

    let dir = unit_dir()?;
    std::fs::create_dir_all(&dir)?;

    let unit = &spec.task_name;
    let service_name = format!("dev.odysync.{unit}.service");
    let timer_name = format!("dev.odysync.{unit}.timer");

    let exec_start = std::iter::once(&exe)
        .chain(args.iter())
        .map(|a| systemd_quoted(a))
        .collect::<Vec<_>>()
        .join(" ");

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

/// See the macOS variant: `Result<()>` to match Windows.
#[cfg(target_os = "linux")]
pub async fn remove_schedule(task_name: &str) -> Result<()> {
    check_task_name(task_name)?;
    let dir = unit_dir()?;

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

    Ok(())
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
    Err(Error::Config(
        "scheduling is not supported on this platform".into(),
    ))
}
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub async fn remove_schedule(_task_name: &str) -> Result<()> {
    Err(Error::Config(
        "scheduling is not supported on this platform".into(),
    ))
}
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub async fn schedule_exists(_task_name: &str) -> bool {
    false
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Parse "HH:MM" into (hour, minute).
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
    fn spec(command: ScheduledCommand) -> ScheduleSpec {
        ScheduleSpec {
            frequency: ScheduleFrequency::Daily,
            time: "09:00".into(),
            task_name: DEFAULT_TASK_NAME.into(),
            extra_args: vec!["--restore-point".into()],
            program: None,
            command,
        }
    }

    #[test]
    fn a_schedule_runs_apply_or_care_unattended() {
        assert_eq!(
            spec(ScheduledCommand::Apply).args(),
            ["apply", "--yes", "--unattended", "--restore-point"]
        );
        assert_eq!(
            spec(ScheduledCommand::Care).args(),
            ["care", "--yes", "--restore-point"]
        );
    }

    /// A run must not be lost because the computer was off, and an installer
    /// must not be stopped because a laptop was unplugged.
    #[test]
    fn the_task_survives_a_missed_start_and_a_pulled_plug() {
        let mut spec = spec(ScheduledCommand::Care);
        spec.extra_args = vec!["--config".into(), r"C:\R&D <x>\c.json".into()];
        let now = jiff::civil::date(2026, 11, 25).at(15, 0, 0, 0);
        let xml = task_xml(&spec, r"C:\Program Files\Odysync\odysync.exe", now).unwrap();
        let doc =
            roxmltree::Document::parse(xml.split_once("?>").expect("declaration").1.trim_start())
                .expect("well-formed");
        let text = |name: &str| {
            let hits: Vec<_> = doc
                .descendants()
                .filter(|n| n.tag_name().name() == name)
                .collect();
            assert_eq!(hits.len(), 1, "{name}");
            hits[0].text().unwrap_or_default().to_string()
        };
        assert_eq!(text("StartWhenAvailable"), "true");
        assert_eq!(text("StopIfGoingOnBatteries"), "false");
        assert_eq!(text("AllowHardTerminate"), "false");
        assert_eq!(text("MultipleInstancesPolicy"), "IgnoreNew");
        assert_eq!(text("RunLevel"), "LeastPrivilege");
        // Chosen at three in the afternoon: tomorrow, not right now.
        assert_eq!(text("StartBoundary"), "2026-11-26T09:00:00");
        assert_eq!(text("DaysInterval"), "1");
        assert_eq!(text("Command"), r"C:\Program Files\Odysync\odysync.exe");
        assert_eq!(
            text("Arguments"),
            r#"care --yes --config "C:\R&D <x>\c.json""#
        );

        spec.frequency = ScheduleFrequency::Weekly;
        assert!(task_xml(&spec, "x", now).unwrap().contains("<Monday />"));
        spec.time = "15:01".into();
        assert!(
            task_xml(&spec, "x", now)
                .unwrap()
                .contains("2026-11-25T15:01:00")
        );
        spec.time = "25:00".into();
        assert!(task_xml(&spec, "x", now).is_err());
    }

    #[test]
    fn arguments_survive_the_schedulers_command_lines() {
        assert_eq!(windows_quoted("--yes"), "--yes");
        assert_eq!(windows_quoted("a b"), r#""a b""#);
        assert_eq!(windows_quoted(r"C:\a b\"), r#""C:\a b\\""#);
        assert_eq!(windows_quoted(r#"a"b"#), r#""a\"b""#);
        assert_eq!(windows_quoted(r#"a\"b"#), r#""a\\\"b""#);
        assert_eq!(windows_quoted(""), r#""""#);
        assert_eq!(systemd_quoted("--yes"), r#""--yes""#);
        assert_eq!(systemd_quoted("/home/x y/50%$A"), r#""/home/x y/50%%$$A""#);
        assert_eq!(systemd_quoted(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    #[test]
    fn a_schedule_name_is_only_a_name() {
        for ok in ["Odysync", "Odysync weekly", "care_2", "a-b"] {
            assert!(check_task_name(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "..",
            "../x",
            r"Microsoft\Windows\x",
            "a/b",
            "a.b",
            "a\nb",
        ] {
            assert!(check_task_name(bad).is_err(), "{bad:?}");
        }
    }

    /// Creates, finds and removes a real task. Never run on a computer
    /// somebody uses; CI runs it on a throw-away one.
    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "registers a scheduled task; CI only"]
    async fn a_real_task_can_be_created_found_and_removed() {
        assert!(
            std::env::var_os("GITHUB_ACTIONS").is_some(),
            "this test changes the machine and only runs in CI"
        );
        let mut spec = spec(ScheduledCommand::Care);
        spec.task_name = "Odysync-ci-test".into();
        spec.program = Some(r"C:\Windows\System32\whoami.exe".into());
        create_schedule(&spec).await.expect("create");
        assert!(schedule_exists(&spec.task_name).await);
        remove_schedule(&spec.task_name).await.expect("remove");
        assert!(!schedule_exists(&spec.task_name).await);
    }

    #[test]
    fn a_spec_saved_before_care_existed_still_means_apply() {
        let old = r#"{"frequency":"daily","time":"09:00","task_name":"Odysync","extra_args":[]}"#;
        let spec: ScheduleSpec = serde_json::from_str(old).unwrap();
        assert_eq!(spec.command, ScheduledCommand::Apply);
    }
}
