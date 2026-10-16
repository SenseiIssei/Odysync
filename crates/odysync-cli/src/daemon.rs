//! Background daemon mode: scan, notify, and optionally apply on a timer.
//!
//! `odysync daemon` runs in the background and periodically checks for updates.
//! When updates are found it emits a notification (via the OS notification
//! system when run under the GUI, or a log line when headless). If `--apply`
//! is set it installs them automatically; otherwise it just reports.

use std::time::Duration;

use anyhow::{Context, Result};
use odysync_core::config::Config;
use odysync_core::platform;
use odysync_core::report::RunReport;
use odysync_core::runner::Runner;
use odysync_core::Backend;

/// Daemon options from the CLI.
pub struct DaemonOpts {
    /// Check interval in minutes.
    pub interval_minutes: u32,
    /// Automatically apply updates without asking.
    pub auto_apply: bool,
    /// Create a restore point before auto-applying (Windows).
    pub restore_point: bool,
    /// Run once and exit (for testing or scheduled invocations).
    pub once: bool,
    /// Scan even when the machine is on battery.
    ///
    /// Off by default. A scan spawns one process per detected backend — over
    /// thirty on a well-equipped Windows machine — and doing that every hour
    /// on battery is a laptop problem, not a feature. An explicit `--once`
    /// run always proceeds: the user asked for it directly.
    pub on_battery: bool,
}

/// Run the daemon loop.
pub async fn run(opts: &DaemonOpts, config_path: &std::path::Path) -> Result<u8> {
    let interval = Duration::from_secs(opts.interval_minutes as u64 * 60);

    // Logged once at startup rather than every cycle, so the target in
    // ROADMAP.md §4 is measured rather than assumed — and so a regression
    // shows up in a user's log without them having to instrument anything.
    if let Some(rss) = platform::resident_memory_bytes() {
        tracing::info!(
            bytes = rss,
            mb = rss as f64 / (1024.0 * 1024.0),
            "daemon resident memory at startup"
        );
    }

    loop {
        // An explicit one-shot run is the user asking directly, so it is never
        // deferred; only the unattended loop waits for mains power.
        if !opts.on_battery && !opts.once {
            let source = platform::power_source();
            if !source.allows_background_work() {
                tracing::info!(
                    ?source,
                    "on battery; skipping this scan. Pass --on-battery to scan anyway"
                );
                tokio::time::sleep(interval).await;
                continue;
            }
        }

        let mut config = Config::load(config_path)
            .with_context(|| format!("loading {}", config_path.display()))?;
        config.policy.elevated = platform::is_elevated();

        let backends = odysync_backends::detect_backends(&config).await;
        let candidates = scan_all(&backends).await;
        let plan = config.policy.plan(candidates);
        let actionable = plan.iter().filter(|p| p.is_actionable()).count();

        let mut failed_count = 0u8;

        if actionable > 0 {
            tracing::info!(count = actionable, "updates available");

            if opts.auto_apply {
                tracing::info!("auto-applying updates");
                let refs: Vec<&dyn Backend> = backends.iter().map(|b| b.as_ref()).collect();
                let mut runner = Runner::new(refs, false);
                let mut report = RunReport::new();
                let restore = opts.restore_point || config.restore_point;
                runner.run(&plan, &mut report, restore).await;
                report.finish();

                tracing::info!(
                    updated = report.updated(),
                    failed = report.failed(),
                    skipped = report.skipped(),
                    reboot = report.reboot_required,
                    "apply complete"
                );

                if report.failed() > 0 {
                    failed_count = 1;
                    tracing::warn!(
                        failed = report.failed(),
                        "some updates failed; will retry on next interval"
                    );
                }
            } else {
                tracing::info!(
                    "{actionable} updates available; use --apply to install automatically"
                );
            }
        } else {
            tracing::debug!("no updates available");
        }

        if opts.once {
            return Ok(failed_count);
        }

        tracing::debug!(secs = interval.as_secs(), "sleeping until next check");
        tokio::time::sleep(interval).await;
    }
}

async fn scan_all(backends: &[Box<dyn Backend>]) -> Vec<odysync_core::model::UpdateCandidate> {
    let results = futures::future::join_all(backends.iter().map(|b| async move {
        match b.scan().await {
            Ok(found) => {
                tracing::info!(backend = %b.kind(), count = found.len(), "scan complete");
                found
            }
            Err(e) => {
                tracing::warn!(backend = %b.kind(), error = %e, "scan failed");
                Vec::new()
            }
        }
    }))
    .await;

    results.into_iter().flatten().collect()
}
