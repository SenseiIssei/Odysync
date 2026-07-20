//! Background daemon mode: scan, notify, and optionally apply on a timer.
//!
//! `sensei daemon` runs in the background and periodically checks for updates.
//! When updates are found it emits a notification (via the OS notification
//! system when run under the GUI, or a log line when headless). If `--apply`
//! is set it installs them automatically; otherwise it just reports.

use std::time::Duration;

use anyhow::{Context, Result};
use sensei_core::config::Config;
use sensei_core::platform;
use sensei_core::report::RunReport;
use sensei_core::runner::Runner;
use sensei_core::Backend;

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
}

/// Run the daemon loop.
pub async fn run(opts: &DaemonOpts, config_path: &std::path::Path) -> Result<u8> {
    let interval = Duration::from_secs(opts.interval_minutes as u64 * 60);

    loop {
        let mut config = Config::load(config_path)
            .with_context(|| format!("loading {}", config_path.display()))?;
        config.policy.elevated = platform::is_elevated();

        let backends = sensei_backends::detect_backends(&config).await;
        let candidates = scan_all(&backends).await;
        let plan = config.policy.plan(candidates);
        let actionable = plan.iter().filter(|p| p.is_actionable()).count();

        if actionable > 0 {
            tracing::info!(count = actionable, "updates available");

            if opts.auto_apply {
                tracing::info!("auto-applying updates");
                let refs: Vec<&dyn Backend> = backends.iter().map(|b| b.as_ref()).collect();
                let runner = Runner::new(refs, false);
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
                    return Ok(1);
                }
            } else {
                tracing::info!("{actionable} updates available; use --apply to install automatically");
            }
        } else {
            tracing::debug!("no updates available");
        }

        if opts.once {
            return Ok(0);
        }

        tracing::debug!(secs = interval.as_secs(), "sleeping until next check");
        tokio::time::sleep(interval).await;
    }
}

async fn scan_all(backends: &[Box<dyn Backend>]) -> Vec<sensei_core::model::UpdateCandidate> {
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
