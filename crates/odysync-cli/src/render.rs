//! Terminal rendering.
//!
//! Colour is opt-out via `NO_COLOR` and is suppressed automatically when stdout
//! is not a terminal, so piping to a file or a log collector stays clean.

use std::io::IsTerminal;

use odysync_core::model::{ApplyOutcome, PlannedUpdate};
use odysync_core::report::RunReport;

/// A byte count the way people read them: `1.4 GB`, `820 KB`, `12 B`.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// `odysync components`.
pub fn components(list: &[odysync_backends::components::ComponentStatus], style: &Style) -> String {
    use odysync_backends::components::ComponentState;
    let mut out = String::new();
    if list.is_empty() {
        out.push_str("No runtime components are tracked on this platform.\n");
        return out;
    }
    out.push_str(&style.bold("Runtime components\n\n"));
    for c in list {
        let state = match c.state {
            ComponentState::Missing => style.dim("missing"),
            ComponentState::Installed => "installed".to_string(),
            ComponentState::Outdated => style.bold("update available"),
            ComponentState::OutOfSupport => style.bold("out of support"),
        };
        let versions = match (&c.installed, &c.latest, c.state) {
            (Some(i), Some(l), ComponentState::Outdated) => format!("{i} -> {l}"),
            (Some(i), _, _) => i.clone(),
            (None, _, _) => String::new(),
        };
        out.push_str(&format!(
            "  {:<28} {:<52} {:<22} {}\n",
            c.id, c.name, versions, state
        ));
    }
    out.push_str(&format!(
        "\n{}\n",
        style.dim(
            "Install or repair one with `odysync components install <id>` in an elevated terminal."
        )
    ));
    out
}

/// `odysync large-files`.
pub fn large_files(report: &odysync_backends::cleanup::LargeFileReport, style: &Style) -> String {
    let mut out = String::new();
    out.push_str(&style.bold("Largest files in your home folder\n\n"));
    if report.files.is_empty() {
        out.push_str("  Nothing above the size threshold.\n");
    }
    for f in &report.files {
        out.push_str(&format!(
            "  {:>10}  {}\n",
            human_bytes(f.bytes),
            f.path.display()
        ));
    }
    out.push_str(&format!(
        "\n{}\n",
        style.dim(&format!(
            "{} files looked at{}. Nothing was deleted — decide for yourself what to remove.",
            report.scanned,
            if report.truncated {
                "; stopped early at the time limit, so larger files may exist"
            } else {
                ""
            }
        ))
    ));
    out
}

pub struct Style {
    colour: bool,
}

impl Style {
    pub fn detect() -> Self {
        // https://no-color.org — any non-empty value disables colour.
        let disabled = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        Self {
            colour: !disabled && std::io::stdout().is_terminal(),
        }
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.colour {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    pub fn bold(&self, t: &str) -> String {
        self.paint("1", t)
    }
    pub fn dim(&self, t: &str) -> String {
        self.paint("2", t)
    }
    pub fn green(&self, t: &str) -> String {
        self.paint("32", t)
    }
    pub fn yellow(&self, t: &str) -> String {
        self.paint("33", t)
    }
    pub fn red(&self, t: &str) -> String {
        self.paint("31", t)
    }
}

/// Render the plan as a table, actionable updates first.
pub fn plan_table(plan: &[PlannedUpdate], style: &Style, show_skipped: bool) -> String {
    let mut out = String::new();

    let (actionable, blocked): (Vec<_>, Vec<_>) = plan.iter().partition(|p| p.is_actionable());

    if actionable.is_empty() {
        out.push_str(&style.dim("No updates are ready to install.\n"));
    } else {
        out.push_str(&style.bold(&format!(
            "{} update{} ready\n\n",
            actionable.len(),
            if actionable.len() == 1 { "" } else { "s" }
        )));

        let name_w = actionable
            .iter()
            .map(|p| p.candidate.name.chars().count())
            .max()
            .unwrap_or(4)
            .clamp(4, 40);

        for p in &actionable {
            let c = &p.candidate;
            out.push_str(&format!(
                "  {:<name_w$}  {} {} {}  {}\n",
                truncate(&c.name, name_w),
                style.dim(c.installed.raw()),
                style.dim("->"),
                style.green(c.available.raw()),
                style.dim(&format!("[{}]", c.id.backend)),
                name_w = name_w
            ));
        }
    }

    if show_skipped && !blocked.is_empty() {
        out.push('\n');
        out.push_str(&style.bold(&format!("{} skipped\n\n", blocked.len())));
        for p in &blocked {
            let reason = p
                .blocked_by
                .as_ref()
                .expect("blocked entries carry a reason");
            out.push_str(&format!(
                "  {:<40}  {}\n",
                truncate(&p.candidate.name, 40),
                style.yellow(&reason.to_string())
            ));
        }
    } else if !blocked.is_empty() {
        out.push_str(&style.dim(&format!(
            "\n{} package(s) skipped by policy. Run with --show-skipped to see why.\n",
            blocked.len()
        )));
    }

    out
}

/// Render the post-run summary.
pub fn summary(report: &RunReport, style: &Style) -> String {
    let mut out = String::new();
    out.push('\n');

    for entry in &report.entries {
        let (mark, text) = match &entry.outcome {
            ApplyOutcome::Updated { from, to } => (
                style.green("ok"),
                format!("{} {} -> {}", entry.name, from, to),
            ),
            ApplyOutcome::DidNotConverge { expected, actual } => (
                style.red("!!"),
                format!(
                    "{} reported success but is at {actual}, expected {expected}",
                    entry.name
                ),
            ),
            ApplyOutcome::VerificationFailed { detail } => (
                style.red("!!"),
                format!("{} failed verification: {detail}", entry.name),
            ),
            ApplyOutcome::Failed { detail } => {
                (style.red("!!"), format!("{} failed: {detail}", entry.name))
            }
            // Skips are already shown in the plan; keep the summary focused.
            ApplyOutcome::Skipped { .. } => continue,
        };
        out.push_str(&format!("  {mark}  {text}\n"));
    }

    out.push_str(&format!(
        "\n{}  {} updated, {} failed, {} skipped\n",
        style.bold("Summary:"),
        style.green(&report.updated().to_string()),
        if report.failed() > 0 {
            style.red(&report.failed().to_string())
        } else {
            report.failed().to_string()
        },
        style.dim(&report.skipped().to_string()),
    ));

    if report.reboot_required {
        out.push_str(&style.yellow("\nA reboot is required to finish applying updates.\n"));
    }

    out
}

/// Truncate to `max` display characters, with an ellipsis when cut.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let keep: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{keep}…")
    }
}

/// `odysync health`.
pub fn health(report: &odysync_backends::device_health::HealthReport, style: &Style) -> String {
    use odysync_backends::device_health::HealthState;
    use std::fmt::Write;

    let mark = |state: HealthState| match state {
        HealthState::Good => style.green("ok  "),
        HealthState::Warning => style.yellow("warn"),
        HealthState::Bad => style.red("BAD "),
        HealthState::Unknown => style.dim("?   "),
    };

    let mut out = String::new();
    let _ = writeln!(out, "{}", style.bold("Disks\n"));
    if report.disks.is_empty() {
        let _ = writeln!(out, "  {}", style.dim("No disks could be read."));
    }
    for d in &report.disks {
        let mut facts = vec![d.kind.clone(), human_bytes(d.size_bytes)];
        if let Some(t) = d.temperature_c {
            facts.push(format!("{t} °C"));
        }
        if let Some(w) = d.wear_percent {
            facts.push(format!("{w}% worn"));
        }
        if let Some(h) = d.power_on_hours {
            facts.push(format!("{h} h powered on"));
        }
        let _ = writeln!(
            out,
            "  {}  {}  {}",
            mark(d.state),
            style.bold(&d.name),
            style.dim(&facts.join(" · "))
        );
        let _ = writeln!(out, "        {}", d.summary);
    }

    if let Some(b) = &report.battery {
        let _ = writeln!(out, "\n{}\n", style.bold("Battery"));
        let mut facts = Vec::new();
        if let (Some(full), Some(design)) = (b.full_capacity, b.design_capacity) {
            facts.push(format!("{full} of {design} {}", b.capacity_unit));
        }
        if let Some(c) = b.cycle_count {
            facts.push(format!("{c} cycles"));
        }
        let _ = writeln!(
            out,
            "  {}  {}  {}",
            mark(b.state),
            b.summary,
            style.dim(&facts.join(" · "))
        );
    }

    for note in &report.notes {
        let _ = writeln!(out, "\n{}", style.dim(note));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_counts_are_humanised() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn short_strings_are_untouched() {
        assert_eq!(truncate("Firefox", 20), "Firefox");
        assert_eq!(truncate("exact", 5), "exact");
    }

    #[test]
    fn long_strings_are_cut_with_an_ellipsis() {
        assert_eq!(truncate("abcdefghij", 5), "abcd…");
        assert_eq!(truncate("abcdefghij", 5).chars().count(), 5);
    }

    #[test]
    fn multibyte_names_are_cut_by_character_not_byte() {
        // Slicing by byte here would panic on a char boundary.
        let cut = truncate("メモ帳アプリケーション", 5);
        assert_eq!(cut.chars().count(), 5);
    }

    #[test]
    fn style_emits_no_escapes_when_colour_is_off() {
        let plain = Style { colour: false };
        assert_eq!(plain.green("ok"), "ok");
        assert!(!plain.bold("x").contains('\x1b'));
    }

    #[test]
    fn style_emits_escapes_when_colour_is_on() {
        let coloured = Style { colour: true };
        assert!(coloured.green("ok").contains("\x1b[32m"));
    }
}
