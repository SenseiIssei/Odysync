//! Terminal rendering.
//!
//! Colour is opt-out via `NO_COLOR` and is suppressed automatically when stdout
//! is not a terminal, so piping to a file or a log collector stays clean.

use std::io::IsTerminal;

use odysync_core::model::{ApplyOutcome, PlannedUpdate};
use odysync_core::report::RunReport;

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

#[cfg(test)]
mod tests {
    use super::*;

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
