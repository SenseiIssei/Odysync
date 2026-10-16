//! Start-at-sign-in registration for Odysync itself.
//!
//! On Linux this is an XDG autostart entry in `~/.config/autostart`, and on
//! macOS a LaunchAgent in `~/Library/LaunchAgents` — both per user, both
//! without root, both removed by deleting one file. Windows is described
//! below.
//!
//! Registration uses the *per-user* Run key
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. That deliberately
//! avoids both a scheduled task and `HKLM`: either would need elevation, and a
//! desktop app that asks for admin rights just to tick "start with Windows" is
//! a bad trade. The per-user key needs no privileges at all and is removed
//! automatically when the user profile goes away.
//!
//! The registry work is done by shelling out through
//! [`odysync_core::proc::powershell`], matching how the rest of this crate
//! talks to Windows. Every value interpolated into a script goes through
//! [`ps_quote`] first — the executable path is attacker-influenced in the sense
//! that it can contain quotes, and an unescaped `'` would close the literal and
//! let the remainder run as PowerShell.

use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Registry key holding per-user startup commands.
#[cfg(windows)]
const RUN_KEY: &str = r"HKCU:\Software\Microsoft\Windows\CurrentVersion\Run";

/// Value name Odysync writes under the Run key.
pub const RUN_VALUE_NAME: &str = "Odysync";

/// Flag appended to the startup command when the app should start hidden.
pub const MINIMIZED_FLAG: &str = "--minimized";

#[cfg(windows)]
const POWERSHELL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Whether Odysync starts with Windows, and whether it starts minimized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AutostartConfig {
    /// A Run entry for Odysync exists.
    pub enabled: bool,
    /// The registered command carries the "start minimized" flag.
    pub minimized: bool,
}

/// A startup command line split back into its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutostartCommand {
    /// Executable path, with any surrounding quotes removed.
    pub exe: String,
    /// Whether the command asks for a minimized start.
    pub minimized: bool,
}

/// Escape a string as a PowerShell single-quoted literal.
///
/// Inside `'...'` PowerShell performs no expansion whatsoever, so the only
/// character that needs handling is the quote itself, which is escaped by
/// doubling it. This is the single defence against script injection through an
/// executable path, so it is used for *every* interpolated value.
#[cfg_attr(not(windows), allow(dead_code))]
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Build the command line stored in the Run key for `exe`.
///
/// The path is always quoted: `C:\Program Files\...` contains spaces, and an
/// unquoted value makes Windows try `C:\Program.exe` first.
pub fn build_run_command(exe: &Path, minimized: bool) -> String {
    build_run_command_str(&exe.display().to_string(), minimized)
}

/// [`build_run_command`] over an already-stringified path (kept separate so the
/// pure logic is testable without touching the filesystem).
pub fn build_run_command_str(exe: &str, minimized: bool) -> String {
    if minimized {
        format!("\"{exe}\" {MINIMIZED_FLAG}")
    } else {
        format!("\"{exe}\"")
    }
}

/// Parse a Run-key value back into its executable path and flags.
///
/// Handles three shapes:
///   * `"C:\path with spaces\odysync.exe" --minimized` — what we write today
///   * `"C:\path\odysync.exe"` — quoted, no flag
///   * `C:\path\odysync.exe` — unquoted, written by an older version
///
/// For the unquoted form the path itself may contain spaces, so trailing
/// switch-like tokens are peeled off the end and everything before them is
/// treated as the path.
pub fn parse_run_command(value: &str) -> AutostartCommand {
    let value = value.trim();

    let (exe, args) = if let Some(rest) = value.strip_prefix('"') {
        match rest.find('"') {
            Some(end) => (rest[..end].to_string(), rest[end + 1..].to_string()),
            // Unterminated quote: treat the whole remainder as the path.
            None => (rest.to_string(), String::new()),
        }
    } else {
        let mut tokens: Vec<&str> = value.split_whitespace().collect();
        let mut flags: Vec<&str> = Vec::new();
        while let Some(last) = tokens.last() {
            if last.starts_with('-') || last.starts_with('/') {
                flags.insert(0, last);
                tokens.pop();
            } else {
                break;
            }
        }
        (tokens.join(" "), flags.join(" "))
    };

    let minimized = args.split_whitespace().any(is_minimized_flag);

    AutostartCommand { exe, minimized }
}

/// Accept the spellings an older build (or a hand-edited value) might use.
fn is_minimized_flag(token: &str) -> bool {
    let token = token.trim_start_matches(['-', '/']);
    token.eq_ignore_ascii_case("minimized") || token.eq_ignore_ascii_case("min")
}

/// Path of the executable to register.
#[cfg(windows)]
fn current_exe() -> Result<PathBuf> {
    std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("could not resolve the current executable path: {e}"))
}

// ── Windows implementation ──────────────────────────────────────────────────

/// Current autostart registration state.
#[cfg(windows)]
pub async fn status() -> Result<AutostartConfig> {
    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
try {{
    $item = Get-ItemProperty -Path {key} -Name {name} -ErrorAction Stop
    [Console]::Out.WriteLine('ODYSYNC_VALUE=' + $item.{name_bare})
}} catch {{
    [Console]::Out.WriteLine('ODYSYNC_ABSENT')
}}"#,
        key = ps_quote(RUN_KEY),
        name = ps_quote(RUN_VALUE_NAME),
        name_bare = ps_quote(RUN_VALUE_NAME),
    );

    let out = odysync_core::proc::powershell(&script, POWERSHELL_TIMEOUT).await?;

    let value = out
        .stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("ODYSYNC_VALUE="));

    Ok(match value {
        Some(v) if !v.trim().is_empty() => AutostartConfig {
            enabled: true,
            minimized: parse_run_command(v).minimized,
        },
        _ => AutostartConfig::default(),
    })
}

/// Register the current executable to start with Windows.
#[cfg(windows)]
pub async fn enable(minimized: bool) -> Result<()> {
    let exe = current_exe()?;
    let command = build_run_command(&exe, minimized);

    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
$key = {key}
if (-not (Test-Path $key)) {{ New-Item -Path $key -Force | Out-Null }}
New-ItemProperty -Path $key -Name {name} -Value {value} -PropertyType String -Force | Out-Null
[Console]::Out.WriteLine('ODYSYNC_OK')"#,
        key = ps_quote(RUN_KEY),
        name = ps_quote(RUN_VALUE_NAME),
        value = ps_quote(&command),
    );

    let out = odysync_core::proc::powershell(&script, POWERSHELL_TIMEOUT).await?;
    if out.stdout.contains("ODYSYNC_OK") {
        tracing::info!(minimized, command = %command, "registered autostart entry");
        Ok(())
    } else {
        let detail = out.stderr.trim();
        anyhow::bail!(
            "could not write the startup entry{}",
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        )
    }
}

/// Remove the autostart registration. Succeeds when there was nothing to remove.
#[cfg(windows)]
pub async fn disable() -> Result<()> {
    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
$key = {key}
if (Test-Path $key) {{
    Remove-ItemProperty -Path $key -Name {name} -ErrorAction SilentlyContinue
}}
[Console]::Out.WriteLine('ODYSYNC_OK')"#,
        key = ps_quote(RUN_KEY),
        name = ps_quote(RUN_VALUE_NAME),
    );

    let out = odysync_core::proc::powershell(&script, POWERSHELL_TIMEOUT).await?;
    if out.stdout.contains("ODYSYNC_OK") {
        tracing::info!("removed autostart entry");
        Ok(())
    } else {
        let detail = out.stderr.trim();
        anyhow::bail!(
            "could not remove the startup entry{}",
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        )
    }
}

// ── Linux and macOS ─────────────────────────────────────────────────────────

/// The XDG autostart entry for `exe`.
pub fn desktop_entry(exe: &str, minimized: bool) -> String {
    // Exec quoting per the Desktop Entry spec: double quotes, with `"`, `` ` ``,
    // `$` and `\` escaped inside them.
    let quoted: String = exe
        .chars()
        .flat_map(|c| match c {
            '"' | '`' | '$' | '\\' => vec!['\\', c],
            other => vec![other],
        })
        .collect();
    let flag = if minimized {
        format!(" {MINIMIZED_FLAG}")
    } else {
        String::new()
    };
    format!(
        "[Desktop Entry]\nType=Application\nName=Odysync\nComment=Safe software and driver updates\n\
         Exec=\"{quoted}\"{flag}\nX-GNOME-Autostart-enabled=true\n"
    )
}

/// The LaunchAgent property list for `exe`.
pub fn launch_agent(exe: &str, minimized: bool) -> Vec<u8> {
    let mut args = vec![plist::Value::String(exe.to_string())];
    if minimized {
        args.push(plist::Value::String(MINIMIZED_FLAG.to_string()));
    }
    let mut dict = plist::Dictionary::new();
    dict.insert(
        "Label".into(),
        plist::Value::String(LAUNCH_AGENT_LABEL.into()),
    );
    dict.insert("ProgramArguments".into(), plist::Value::Array(args));
    dict.insert("RunAtLoad".into(), plist::Value::Boolean(true));
    let mut out = Vec::new();
    // Writing a dictionary of strings and booleans cannot fail.
    let _ = plist::Value::Dictionary(dict).to_writer_xml(&mut out);
    out
}

/// launchd label for Odysync's LaunchAgent.
pub const LAUNCH_AGENT_LABEL: &str = "com.senseiissei.odysync";

#[cfg(not(windows))]
fn registration_file() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("no home directory"))?;
    Ok(if cfg!(target_os = "macos") {
        home.join("Library")
            .join("LaunchAgents")
            .join(format!("{LAUNCH_AGENT_LABEL}.plist"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".config"))
            .join("autostart")
            .join("odysync.desktop")
    })
}

#[cfg(not(windows))]
use std::path::PathBuf;

#[cfg(not(windows))]
pub async fn status() -> Result<AutostartConfig> {
    let path = registration_file()?;
    let Ok(bytes) = std::fs::read(&path) else {
        return Ok(AutostartConfig::default());
    };
    let text = String::from_utf8_lossy(&bytes);
    let enabled = if cfg!(target_os = "macos") {
        true
    } else {
        crate::startup_items::parse_desktop_entry(&text).enabled()
    };
    Ok(AutostartConfig {
        enabled,
        minimized: text.contains(MINIMIZED_FLAG),
    })
}

#[cfg(not(windows))]
pub async fn enable(minimized: bool) -> Result<()> {
    let exe = std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("could not resolve the current executable path: {e}"))?;
    let exe = exe.display().to_string();
    let path = registration_file()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = if cfg!(target_os = "macos") {
        launch_agent(&exe, minimized)
    } else {
        desktop_entry(&exe, minimized).into_bytes()
    };
    std::fs::write(&path, content)?;
    tracing::info!(minimized, path = %path.display(), "registered autostart entry");
    Ok(())
}

#[cfg(not(windows))]
pub async fn disable() -> Result<()> {
    let path = registration_file()?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_desktop_entry_quotes_the_path_and_carries_the_flag() {
        let entry = desktop_entry("/opt/My Apps/odysync-gui", true);
        let parsed = crate::startup_items::parse_desktop_entry(&entry);
        assert_eq!(parsed.name.as_deref(), Some("Odysync"));
        assert_eq!(
            parsed.exec.as_deref(),
            Some("\"/opt/My Apps/odysync-gui\" --minimized")
        );
        assert!(parsed.enabled());
        // Characters the spec reserves inside quotes are escaped.
        assert!(desktop_entry("/x/$HOME`id`", false).contains("\\$HOME\\`id\\`"));
    }

    #[test]
    fn the_launch_agent_runs_at_login_with_the_right_arguments() {
        let bytes = launch_agent("/Applications/Odysync.app/Contents/MacOS/odysync-gui", true);
        let (label, command, disabled) =
            crate::startup_items::parse_launch_agent(&bytes).expect("a valid agent");
        assert_eq!(label, LAUNCH_AGENT_LABEL);
        assert_eq!(
            command,
            "/Applications/Odysync.app/Contents/MacOS/odysync-gui --minimized"
        );
        assert!(!disabled);
    }

    use super::*;

    #[test]
    fn ps_quote_neutralises_injection() {
        assert_eq!(ps_quote("plain"), "'plain'");
        assert_eq!(ps_quote("it's"), "'it''s'");
        // The attack this exists for: close the literal, run a second
        // statement, reopen the literal so the script still parses.
        assert_eq!(
            ps_quote(r"C:\x'; Remove-Item C:\ -Recurse -Force; '"),
            r"'C:\x''; Remove-Item C:\ -Recurse -Force; '''"
        );
        // Anything that is not a quote stays literal inside '...'.
        assert_eq!(ps_quote("$(rm -rf /)`n%TEMP%"), "'$(rm -rf /)`n%TEMP%'");
    }

    #[test]
    fn quoted_exe_path_is_injection_safe_end_to_end() {
        let command = build_run_command_str(r"C:\Program Files\O'dysync\app.exe", true);
        let quoted = ps_quote(&command);
        // Every single quote in the payload is doubled, so the literal cannot
        // be terminated early.
        assert!(quoted.starts_with('\''));
        assert!(quoted.ends_with('\''));
        assert!(!quoted[1..quoted.len() - 1].contains("';"));
        assert_eq!(
            quoted,
            r#"'"C:\Program Files\O''dysync\app.exe" --minimized'"#
        );
    }

    #[test]
    fn builds_a_quoted_command_line() {
        assert_eq!(
            build_run_command_str(r"C:\Program Files\Odysync\odysync.exe", false),
            r#""C:\Program Files\Odysync\odysync.exe""#
        );
        assert_eq!(
            build_run_command_str(r"C:\Program Files\Odysync\odysync.exe", true),
            r#""C:\Program Files\Odysync\odysync.exe" --minimized"#
        );
    }

    #[test]
    fn round_trips_paths_with_spaces() {
        for minimized in [false, true] {
            let exe = r"C:\Program Files\Odysync\odysync.exe";
            let parsed = parse_run_command(&build_run_command_str(exe, minimized));
            assert_eq!(parsed.exe, exe);
            assert_eq!(parsed.minimized, minimized);
        }
    }

    #[test]
    fn parses_a_value_written_by_an_older_version() {
        // Older builds wrote the bare, unquoted path with no flag.
        let parsed = parse_run_command(r"C:\Program Files\Odysync\odysync.exe");
        assert_eq!(parsed.exe, r"C:\Program Files\Odysync\odysync.exe");
        assert!(!parsed.minimized);

        // ...and some wrote a slash- or single-dash-style switch.
        let parsed = parse_run_command(r"C:\Odysync\odysync.exe /minimized");
        assert_eq!(parsed.exe, r"C:\Odysync\odysync.exe");
        assert!(parsed.minimized);

        let parsed = parse_run_command(r"C:\Odysync\odysync.exe -min");
        assert_eq!(parsed.exe, r"C:\Odysync\odysync.exe");
        assert!(parsed.minimized);
    }

    #[test]
    fn parses_extra_arguments_without_confusing_the_flag() {
        let parsed = parse_run_command(r#""C:\a b\odysync.exe" --tray --minimized --quiet"#);
        assert_eq!(parsed.exe, r"C:\a b\odysync.exe");
        assert!(parsed.minimized);

        let parsed = parse_run_command(r#""C:\a b\odysync.exe" --tray"#);
        assert_eq!(parsed.exe, r"C:\a b\odysync.exe");
        assert!(!parsed.minimized);

        // A path that merely *contains* the word must not count as the flag.
        let parsed = parse_run_command(r#""C:\minimized\odysync.exe""#);
        assert!(!parsed.minimized);
    }

    #[test]
    fn tolerates_a_blank_or_malformed_value() {
        assert_eq!(parse_run_command("").exe, "");
        assert!(!parse_run_command("").minimized);
        let parsed = parse_run_command(r#""C:\unterminated\odysync.exe --minimized"#);
        assert!(parsed.exe.contains("odysync.exe"));
    }

    /// A real round trip through the filesystem. `XDG_CONFIG_HOME` is only
    /// read by this module and `startup_items`, whose tests do not touch the
    /// environment, so pointing it at a temporary directory is safe here.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[tokio::test]
    async fn linux_registration_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", dir.path());

        assert_eq!(status().await.unwrap(), AutostartConfig::default());
        enable(true).await.unwrap();
        assert_eq!(
            status().await.unwrap(),
            AutostartConfig {
                enabled: true,
                minimized: true
            }
        );
        assert!(dir
            .path()
            .join("autostart")
            .join("odysync.desktop")
            .exists());
        disable().await.unwrap();
        assert_eq!(status().await.unwrap(), AutostartConfig::default());
        // Disabling twice is not an error.
        disable().await.unwrap();

        std::env::remove_var("XDG_CONFIG_HOME");
    }
}
