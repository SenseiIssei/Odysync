//! Programs that start when the user signs in — listed, and switched on or off
//! reversibly.
//!
//! "Off" never means deleted. Each platform has an official way to say "do not
//! start this" that leaves the program's own registration intact, so turning
//! an entry back on is always possible from the same screen:
//!
//! * **Windows** — the `StartupApproved` flags Task Manager itself writes, and
//!   a `.disabled` suffix for shortcuts in the Startup folder.
//! * **Linux** — the XDG autostart spec: a per-user copy of the `.desktop`
//!   file with `Hidden=true`, which overrides the system-wide one.
//! * **macOS** — `launchctl disable` for a LaunchAgent in the user's own
//!   `gui/<uid>` domain, which persists across reboots and needs no root.

use serde::{Deserialize, Serialize};

/// One startup entry, as the front-ends show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartupItem {
    /// Identifies the entry together with `location`; passed back to
    /// [`set_enabled`] unchanged.
    pub name: String,
    /// What to show the user, when it differs from `name`.
    pub label: Option<String>,
    pub command: String,
    /// Where the entry lives: a registry key, a folder, or a LaunchAgents dir.
    pub location: String,
    pub enabled: bool,
}

// ── Linux: XDG autostart (pure) ─────────────────────────────────────────────

/// The fields Odysync reads from a `.desktop` file's `[Desktop Entry]` group.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DesktopEntry {
    pub name: Option<String>,
    pub exec: Option<String>,
    pub hidden: bool,
    /// `X-GNOME-Autostart-enabled=false` — GNOME's own off switch.
    pub gnome_disabled: bool,
}

impl DesktopEntry {
    pub fn enabled(&self) -> bool {
        !self.hidden && !self.gnome_disabled
    }
}

pub fn parse_desktop_entry(content: &str) -> DesktopEntry {
    let mut entry = DesktopEntry::default();
    let mut in_group = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            continue;
        }
        if !in_group || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "Name" => entry.name = Some(value.to_string()),
            "Exec" => entry.exec = Some(value.to_string()),
            "Hidden" => entry.hidden = value.eq_ignore_ascii_case("true"),
            "X-GNOME-Autostart-enabled" => {
                entry.gnome_disabled = value.eq_ignore_ascii_case("false")
            }
            _ => {}
        }
    }
    entry
}

/// Rewrite a `.desktop` file so it is (or is not) hidden from autostart.
///
/// Only the `[Desktop Entry]` group is touched, and only the two keys that
/// switch autostart; every other line — translations, comments, actions — is
/// kept verbatim.
pub fn with_autostart(content: &str, enabled: bool) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_group = false;
    let mut wrote = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_group && !wrote {
                out.push(format!("Hidden={}", !enabled));
                wrote = true;
            }
            in_group = trimmed == "[Desktop Entry]";
            out.push(line.to_string());
            continue;
        }
        if in_group {
            let key = trimmed.split('=').next().unwrap_or("").trim();
            if key == "Hidden" {
                if !wrote {
                    out.push(format!("Hidden={}", !enabled));
                    wrote = true;
                }
                continue;
            }
            if key == "X-GNOME-Autostart-enabled" {
                out.push(format!("X-GNOME-Autostart-enabled={enabled}"));
                continue;
            }
        }
        out.push(line.to_string());
    }
    if in_group && !wrote {
        out.push(format!("Hidden={}", !enabled));
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

// ── macOS: launchctl (pure) ─────────────────────────────────────────────────

/// Labels `launchctl print-disabled gui/<uid>` reports as disabled.
///
/// Recent macOS prints `"label" => disabled`; older releases print
/// `"label" => true`. Both are read.
pub fn parse_print_disabled(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let (label, state) = line.split_once("=>")?;
            let state = state.trim();
            if state == "disabled" || state == "true" {
                Some(label.trim().trim_matches('"').to_string())
            } else {
                None
            }
        })
        .filter(|l| !l.is_empty())
        .collect()
}

#[derive(Debug, Deserialize)]
struct LaunchAgent {
    #[serde(rename = "Label")]
    label: Option<String>,
    #[serde(rename = "Program")]
    program: Option<String>,
    #[serde(rename = "ProgramArguments")]
    program_arguments: Option<Vec<String>>,
    #[serde(rename = "RunAtLoad")]
    run_at_load: Option<bool>,
    #[serde(rename = "Disabled")]
    disabled: Option<bool>,
}

/// `(label, command, disabled_in_plist)` for an agent that runs at login.
pub fn parse_launch_agent(bytes: &[u8]) -> Option<(String, String, bool)> {
    let agent: LaunchAgent = plist::from_bytes(bytes).ok()?;
    if agent.run_at_load != Some(true) {
        return None;
    }
    let label = agent.label?;
    let command = agent
        .program_arguments
        .map(|a| a.join(" "))
        .or(agent.program)
        .unwrap_or_default();
    Some((label, command, agent.disabled.unwrap_or(false)))
}

// ── Platform implementations ────────────────────────────────────────────────

pub async fn list() -> anyhow::Result<Vec<StartupItem>> {
    let mut items = imp::list().await?;
    items.sort_by_key(|i| {
        i.label
            .clone()
            .unwrap_or_else(|| i.name.clone())
            .to_lowercase()
    });
    Ok(items)
}

pub async fn set_enabled(name: &str, location: &str, enable: bool) -> anyhow::Result<()> {
    imp::set_enabled(name, location, enable).await
}

#[cfg(windows)]
mod imp {
    use super::StartupItem;

    /// Escape a string as a PowerShell single-quoted literal.
    fn ps_quote(s: &str) -> String {
        format!("'{}'", s.replace('\'', "''"))
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct StartupProgramJson {
        name: String,
        command: Option<String>,
        location: String,
        enabled: bool,
    }

    pub async fn list() -> anyhow::Result<Vec<StartupItem>> {
        // The StartupApproved keys hold a 12-byte blob per entry whose first
        // byte encodes the state: even values are enabled, odd are disabled.
        let script = include_str!("startup_items_list.ps1");
        let out =
            odysync_core::proc::powershell(script, std::time::Duration::from_secs(30)).await?;
        let programs: Vec<StartupProgramJson> = crate::security::parse_ps_json(&out.stdout);
        Ok(programs
            .into_iter()
            .map(|p| StartupItem {
                name: p.name,
                label: None,
                command: p.command.unwrap_or_default(),
                location: p.location,
                enabled: p.enabled,
            })
            .collect())
    }

    pub async fn set_enabled(name: &str, location: &str, enable: bool) -> anyhow::Result<()> {
        let script = format!(
            "$name = {}\n$location = {}\n$enable = ${}\n{}",
            ps_quote(name),
            ps_quote(location),
            if enable { "true" } else { "false" },
            include_str!("startup_items_toggle.ps1"),
        );
        let out =
            odysync_core::proc::powershell(&script, std::time::Duration::from_secs(20)).await?;
        if out.stdout.contains("ODYSYNC_OK") {
            Ok(())
        } else {
            let detail = out.stderr.trim();
            anyhow::bail!(if detail.is_empty() {
                "Failed to change the startup entry.".to_string()
            } else {
                format!("Failed to change the startup entry: {detail}")
            })
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use std::path::PathBuf;

    use super::{StartupItem, parse_desktop_entry, with_autostart};

    fn user_dir() -> Option<PathBuf> {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|c| c.join("autostart"))
    }

    fn system_dirs() -> Vec<PathBuf> {
        let dirs = std::env::var("XDG_CONFIG_DIRS").unwrap_or_else(|_| "/etc/xdg".into());
        dirs.split(':')
            .filter(|d| d.starts_with('/'))
            .map(|d| PathBuf::from(d).join("autostart"))
            .collect()
    }

    fn desktop_files(dir: &PathBuf) -> Vec<(String, PathBuf)> {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.ends_with(".desktop").then(|| (name, e.path()))
            })
            .collect()
    }

    pub async fn list() -> anyhow::Result<Vec<StartupItem>> {
        let user = user_dir();
        let user_files = user.as_ref().map(desktop_files).unwrap_or_default();
        let mut items: Vec<StartupItem> = Vec::new();

        // A per-user file overrides a system file of the same name.
        let mut seen: Vec<String> = Vec::new();
        for (file, path) in &user_files {
            let entry = parse_desktop_entry(&std::fs::read_to_string(path).unwrap_or_default());
            seen.push(file.clone());
            items.push(StartupItem {
                name: file.trim_end_matches(".desktop").to_string(),
                label: entry.name.clone(),
                command: entry.exec.clone().unwrap_or_default(),
                location: user
                    .as_ref()
                    .map(|u| u.display().to_string())
                    .unwrap_or_default(),
                enabled: entry.enabled(),
            });
        }
        for dir in system_dirs() {
            for (file, path) in desktop_files(&dir) {
                if seen.contains(&file) {
                    continue;
                }
                seen.push(file.clone());
                let entry =
                    parse_desktop_entry(&std::fs::read_to_string(&path).unwrap_or_default());
                items.push(StartupItem {
                    name: file.trim_end_matches(".desktop").to_string(),
                    label: entry.name.clone(),
                    command: entry.exec.clone().unwrap_or_default(),
                    location: dir.display().to_string(),
                    enabled: entry.enabled(),
                });
            }
        }
        Ok(items)
    }

    pub async fn set_enabled(name: &str, _location: &str, enable: bool) -> anyhow::Result<()> {
        if name.contains('/') || name.contains("..") || name.is_empty() {
            anyhow::bail!("not a startup entry name: {name}");
        }
        let file = format!("{name}.desktop");
        let user = user_dir().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
        let user_path = user.join(&file);

        // The user's own copy is edited in place; a system entry gets a
        // per-user override, which is exactly how the spec says to disable one
        // without touching files that belong to the system.
        let source = if user_path.exists() {
            std::fs::read_to_string(&user_path)?
        } else {
            let system = system_dirs()
                .into_iter()
                .map(|d| d.join(&file))
                .find(|p| p.exists())
                .ok_or_else(|| anyhow::anyhow!("no startup entry named {name}"))?;
            std::fs::read_to_string(system)?
        };
        std::fs::create_dir_all(&user)?;
        std::fs::write(&user_path, with_autostart(&source, enable))?;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{StartupItem, parse_launch_agent, parse_print_disabled};

    async fn uid() -> anyhow::Result<String> {
        let out = odysync_core::proc::run("id", &["-u"], Duration::from_secs(10)).await?;
        let uid = out.stdout.trim().to_string();
        anyhow::ensure!(
            !uid.is_empty() && uid.bytes().all(|b| b.is_ascii_digit()),
            "could not read the user id"
        );
        Ok(uid)
    }

    fn agent_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join("Library").join("LaunchAgents"));
        }
        dirs.push(PathBuf::from("/Library/LaunchAgents"));
        dirs
    }

    pub async fn list() -> anyhow::Result<Vec<StartupItem>> {
        let uid = uid().await?;
        let disabled = odysync_core::proc::run(
            "launchctl",
            &["print-disabled", &format!("gui/{uid}")],
            Duration::from_secs(20),
        )
        .await
        .map(|o| parse_print_disabled(&o.stdout))
        .unwrap_or_default();

        let mut items = Vec::new();
        for dir in agent_dirs() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("plist") {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                if let Some((label, command, plist_disabled)) = parse_launch_agent(&bytes) {
                    let off = plist_disabled || disabled.contains(&label);
                    items.push(StartupItem {
                        name: label,
                        label: None,
                        command,
                        location: dir.display().to_string(),
                        enabled: !off,
                    });
                }
            }
        }
        Ok(items)
    }

    pub async fn set_enabled(name: &str, _location: &str, enable: bool) -> anyhow::Result<()> {
        // A launchd label is reverse-DNS; anything else is not one of ours.
        anyhow::ensure!(
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')),
            "not a launchd label: {name}"
        );
        let uid = uid().await?;
        let verb = if enable { "enable" } else { "disable" };
        let out = odysync_core::proc::run(
            "launchctl",
            &[verb, &format!("gui/{uid}/{name}")],
            Duration::from_secs(20),
        )
        .await?;
        anyhow::ensure!(
            out.success(),
            "launchctl {verb} failed: {}",
            out.stderr.trim()
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESKTOP: &str = "[Desktop Entry]\n\
        Type=Application\n\
        Name=Nextcloud\n\
        Name[de]=Nextcloud\n\
        Exec=/usr/bin/nextcloud --background\n\
        X-GNOME-Autostart-enabled=true\n\
        \n\
        [Desktop Action Quit]\n\
        Name=Quit\n\
        Hidden=keep-me\n";

    #[test]
    fn a_desktop_entry_is_read_from_its_own_group_only() {
        let e = parse_desktop_entry(DESKTOP);
        assert_eq!(e.name.as_deref(), Some("Nextcloud"));
        assert_eq!(e.exec.as_deref(), Some("/usr/bin/nextcloud --background"));
        assert!(e.enabled());
    }

    #[test]
    fn both_off_switches_are_honoured() {
        assert!(!parse_desktop_entry("[Desktop Entry]\nHidden=true\n").enabled());
        assert!(
            !parse_desktop_entry("[Desktop Entry]\nX-GNOME-Autostart-enabled=false\n").enabled()
        );
        assert!(parse_desktop_entry("[Desktop Entry]\nHidden=false\n").enabled());
    }

    #[test]
    fn disabling_hides_the_entry_and_keeps_everything_else() {
        let off = with_autostart(DESKTOP, false);
        assert!(!parse_desktop_entry(&off).enabled());
        // Other groups and keys survive untouched.
        assert!(off.contains("Name[de]=Nextcloud"));
        assert!(off.contains("[Desktop Action Quit]"));
        assert!(
            off.contains("Hidden=keep-me"),
            "a key in another group was changed"
        );

        let on = with_autostart(&off, true);
        assert!(parse_desktop_entry(&on).enabled());
        assert_eq!(
            on.matches("Hidden=").count(),
            2,
            "Hidden= was duplicated: {on}"
        );
    }

    #[test]
    fn a_file_with_no_hidden_key_gains_one() {
        let off = with_autostart("[Desktop Entry]\nName=X\nExec=x\n", false);
        assert!(off.contains("Hidden=true"));
        assert!(!parse_desktop_entry(&off).enabled());
    }

    #[test]
    fn launchctl_disabled_labels_are_read_in_both_formats() {
        let out = "disabled services = {\n\
                   \t\"com.spotify.webhelper\" => disabled\n\
                   \t\"com.apple.Siri.agent\" => enabled\n\
                   \t\"com.old.style\" => true\n\
                   \t\"com.old.enabled\" => false\n\
                   }\n";
        assert_eq!(
            parse_print_disabled(out),
            vec![
                "com.spotify.webhelper".to_string(),
                "com.old.style".to_string()
            ]
        );
    }

    #[test]
    fn only_agents_that_run_at_login_are_startup_items() {
        let agent = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
  <key>Label</key><string>com.example.helper</string>
  <key>ProgramArguments</key><array><string>/Applications/Example.app/Contents/MacOS/helper</string><string>--quiet</string></array>
  <key>RunAtLoad</key><true/>
</dict></plist>"#;
        let (label, command, disabled) = parse_launch_agent(agent.as_bytes()).unwrap();
        assert_eq!(label, "com.example.helper");
        assert_eq!(
            command,
            "/Applications/Example.app/Contents/MacOS/helper --quiet"
        );
        assert!(!disabled);

        let on_demand = agent.replace("<key>RunAtLoad</key><true/>", "");
        assert!(parse_launch_agent(on_demand.as_bytes()).is_none());
    }
}
