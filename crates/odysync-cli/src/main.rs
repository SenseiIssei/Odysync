//! `odysync` — the cross-platform command line for Odysync.
//!
//! The CLI is a thin shell over `odysync-core`: it gathers candidates from every
//! detected backend, runs them through the policy engine, and applies whatever
//! survives. Every safety decision belongs to the core, so the CLI and the
//! forthcoming GUI cannot drift apart in what they consider safe.

mod daemon;
mod render;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use odysync_core::Backend;
use odysync_core::config::Config;
use odysync_core::maintenance::MaintenanceKind;
use odysync_core::model::UpdateCandidate;
use odysync_core::platform;
use odysync_core::report::RunReport;
use odysync_core::runner::Runner;

use render::Style;

#[derive(Parser)]
#[command(
    name = "odysync",
    version,
    about = "Safe, fast software and driver updates for Windows, macOS and Linux"
)]
struct Cli {
    /// Path to the config file (defaults to the per-user location).
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Emit machine-readable JSON instead of a table.
    #[arg(long, global = true)]
    json: bool,

    /// Increase log detail. Repeat for more.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Look for available updates without changing anything.
    Scan {
        /// Also list packages that policy skipped, and why.
        #[arg(long)]
        show_skipped: bool,

        /// Only ask these backends (repeatable), e.g. `--backend direct`.
        /// Naming an opt-in backend such as `winget` includes it for this run.
        #[arg(long = "backend")]
        backend: Vec<String>,
    },

    /// Install the updates that pass every safety check.
    Apply {
        /// Apply without asking for confirmation.
        #[arg(short, long)]
        yes: bool,

        /// Show what would happen without installing anything.
        #[arg(long)]
        dry_run: bool,

        /// Limit to specific packages (repeatable). Matches id or backend:id.
        #[arg(long = "only")]
        only: Vec<String>,

        /// Only ask these backends (repeatable), e.g. `--backend direct`.
        /// Naming an opt-in backend such as `winget` includes it for this run.
        #[arg(long = "backend")]
        backend: Vec<String>,

        /// Limit to a named profile from the config.
        #[arg(long)]
        profile: Option<String>,

        /// Write a run report here (format determined by --report-format).
        #[arg(long)]
        report: Option<PathBuf>,

        /// Report format: json or text.
        #[arg(long, default_value = "json")]
        report_format: ReportFormat,

        /// Create a system restore point before applying (Windows).
        #[arg(long)]
        restore_point: bool,

        /// Leave out drivers, firmware, system upgrades and developer packages
        /// (pip, npm, cargo ...), which need somebody in front of the machine.
        /// Schedules use this.
        #[arg(long)]
        unattended: bool,
    },

    /// Everything Odysync can do safely without you deciding, in one pass:
    /// install the updates that pass every safety rule, then remove the
    /// cleanup categories that are safe by default. Made for schedules.
    Care {
        /// Run without asking for confirmation.
        #[arg(short, long)]
        yes: bool,

        /// Show what would happen without changing anything.
        #[arg(long)]
        dry_run: bool,
    },

    /// Disk and battery health, as the hardware reports it.
    Health,

    /// List the package managers detected on this machine.
    Backends,

    /// Freeze a package so it is never updated.
    Hold {
        package: String,
        /// Only allow this exact version through.
        #[arg(long)]
        pin: Option<String>,
        /// Why, for your future self.
        #[arg(long)]
        note: Option<String>,
    },

    /// Remove a hold.
    Unhold { package: String },

    /// Defer a whole backend for a number of days.
    ///
    /// A deadline rather than an off switch: the pause lapses on its own, so a
    /// bad week does not turn into a year of missed security updates.
    Pause {
        /// Backend id, e.g. `windows-update`.
        backend: String,
        /// How many days to defer for.
        #[arg(long, default_value = "7")]
        days: u32,
        /// Why, for your future self.
        #[arg(long)]
        note: Option<String>,
    },

    /// Lift a pause before it lapses.
    Resume {
        /// Backend id, e.g. `windows-update`.
        backend: String,
    },

    /// Check whether a newer Odysync has been released.
    ///
    /// Reports only. Odysync will not download and run its own updates while
    /// its releases are unsigned — that is the supply-chain hole the v1
    /// rewrite existed to close.
    SelfCheck,

    /// Show the resolved configuration and where it lives.
    Config,

    /// Run a system maintenance action (not a package update).
    Maintain {
        /// Which action to run.
        #[arg(long = "action", value_enum)]
        action: MaintenanceActionArg,
    },

    /// Schedule automatic update runs.
    Schedule {
        /// How often to run.
        #[arg(long, value_enum)]
        frequency: ScheduleFrequencyArg,
        /// Time in 24-hour HH:MM format.
        #[arg(long, default_value = "09:00")]
        time: String,
        /// Task name (for later removal).
        #[arg(long, default_value = odysync_backends::scheduler::DEFAULT_TASK_NAME)]
        task_name: String,
        /// Extra arguments to pass to `odysync apply` (or `care`), one per
        /// `--arg`: `--arg=--only --arg=vscode`.
        #[arg(long = "arg")]
        extra_args: Vec<String>,

        /// Run automatic care (updates plus the safe cleanup) instead of
        /// updates alone.
        #[arg(long)]
        care: bool,
    },

    /// Remove a scheduled task.
    Unschedule {
        /// Task name to remove.
        #[arg(long, default_value = odysync_backends::scheduler::DEFAULT_TASK_NAME)]
        task_name: String,
    },

    /// Create a diagnostics bundle for troubleshooting.
    Diagnostics {
        /// Output path for the zip bundle.
        #[arg(long = "out", default_value = "odysync-diagnostics.zip")]
        out: PathBuf,
    },

    /// Run in the background, periodically scanning (and optionally applying) updates.
    Daemon {
        /// Check interval in minutes.
        #[arg(long, default_value = "60")]
        interval: u32,

        /// Automatically apply updates without asking.
        #[arg(long)]
        apply: bool,

        /// Create a restore point before auto-applying (Windows).
        #[arg(long)]
        restore_point: bool,

        /// Run a single check and exit (for testing or scheduled invocations).
        #[arg(long)]
        once: bool,

        /// Keep scanning while the machine is on battery.
        ///
        /// Off by default: a scan spawns one process per detected backend, and
        /// doing that hourly on battery drains a laptop for no benefit.
        #[arg(long)]
        on_battery: bool,
    },

    /// Back up and restore the Windows driver store.
    #[cfg(windows)]
    Drivers {
        #[command(subcommand)]
        action: DriverAction,
    },

    /// Measure reclaimable disk space, and optionally reclaim it.
    ///
    /// Scanning is the default. Nothing is deleted without `--apply`, and even
    /// then only the categories that are safe by default unless `--only` names
    /// others.
    Clean {
        /// Actually delete. Without this the command only measures.
        #[arg(long)]
        apply: bool,

        /// Comma-separated category ids instead of the safe defaults.
        ///
        /// `odysync clean` lists the ids. Categories that destroy data —
        /// the Recycle Bin, superseded components — are only ever cleaned when
        /// named here.
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,

        /// Skip the confirmation prompt.
        #[arg(short, long)]
        yes: bool,
    },

    /// Game and app runtimes: Visual C++, DirectX, .NET (Windows).
    ///
    /// Without a subcommand, lists what is installed and what is out of date.
    Components {
        #[command(subcommand)]
        action: Option<ComponentAction>,
    },

    /// Devices Windows reports as not working, with what each problem means.
    Devices,

    /// Processor, memory, graphics and disks of this machine.
    Sysinfo,

    /// Programs that start when you sign in. Turning one off is reversible.
    Startup {
        #[command(subcommand)]
        action: Option<StartupAction>,
    },

    /// List the largest files in your home folder. Reports only; deletes nothing.
    LargeFiles {
        /// How many files to list.
        #[arg(long, default_value_t = 25)]
        limit: usize,

        /// Ignore files smaller than this many megabytes.
        #[arg(long, default_value_t = 100)]
        min_mb: u64,
    },

    /// Report how each volume should be optimised, and optionally do it.
    #[cfg(windows)]
    OptimizeDisk {
        /// Actually run the optimisation.
        #[arg(long)]
        apply: bool,
    },
}

#[derive(clap::Subcommand)]
enum StartupAction {
    /// Let an entry start again. NAME and LOCATION as `odysync startup` lists them.
    Enable { name: String, location: String },
    /// Stop an entry from starting, without removing it.
    Disable { name: String, location: String },
}

#[derive(clap::Subcommand)]
enum ComponentAction {
    /// Install or repair one component from Microsoft, e.g. `vc-redist-x64`.
    /// Needs an elevated terminal.
    Install { id: String },
}

/// Driver-store backup operations.
///
/// Exporting works unelevated; putting a package back does not.
#[cfg(windows)]
#[derive(clap::Subcommand)]
enum DriverAction {
    /// Export every third-party driver package to a new backup.
    Backup {
        /// Why this backup is being taken, recorded in the manifest.
        #[arg(long, default_value = "manual")]
        reason: String,

        /// Back up only these published names, e.g. `oem23.inf`.
        ///
        /// A full export copies the whole driver store — several gigabytes —
        /// so backing up just the package about to be replaced is often what
        /// is actually wanted.
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
    },

    /// List the backups on disk, newest first.
    List,

    /// Re-add a backup's driver packages and install them.
    ///
    /// Not a forced downgrade: Windows ranks driver packages, so a newer one
    /// still in the store can keep winning. Needs administrator rights.
    Rollback {
        /// Backup id, as shown by `odysync drivers list`.
        #[arg(long = "to")]
        to: String,
    },

    /// Delete all but the newest backups.
    Prune {
        /// How many to keep.
        #[arg(long, default_value = "3")]
        keep: usize,
    },
}

#[derive(clap::ValueEnum, Clone, Copy)]
enum MaintenanceActionArg {
    #[value(name = "temp-cleanup")]
    TempCleanup,
    #[value(name = "clean-recycle-bin")]
    CleanRecycleBin,
    #[value(name = "system-health")]
    SystemHealth,
    #[value(name = "startup-programs")]
    StartupPrograms,
}

impl From<MaintenanceActionArg> for MaintenanceKind {
    fn from(arg: MaintenanceActionArg) -> Self {
        match arg {
            MaintenanceActionArg::TempCleanup => MaintenanceKind::TempCleanup,
            MaintenanceActionArg::CleanRecycleBin => MaintenanceKind::CleanRecycleBin,
            MaintenanceActionArg::SystemHealth => MaintenanceKind::SystemHealth,
            MaintenanceActionArg::StartupPrograms => MaintenanceKind::StartupPrograms,
        }
    }
}

#[derive(clap::ValueEnum, Clone, Copy)]
enum ScheduleFrequencyArg {
    #[value(name = "daily")]
    Daily,
    #[value(name = "weekly")]
    Weekly,
}

impl From<ScheduleFrequencyArg> for odysync_backends::scheduler::ScheduleFrequency {
    fn from(arg: ScheduleFrequencyArg) -> Self {
        match arg {
            ScheduleFrequencyArg::Daily => Self::Daily,
            ScheduleFrequencyArg::Weekly => Self::Weekly,
        }
    }
}

#[derive(clap::ValueEnum, Clone, Copy)]
enum ReportFormat {
    Json,
    Text,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    init_logging(cli.verbose);

    match run(cli).await {
        Ok(code) => std::process::ExitCode::from(code),
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::ExitCode::from(2)
        }
    }
}

fn init_logging(verbosity: u8) {
    let level = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    // Logs go to stderr so `--json` on stdout stays parseable.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("odysync={level}").into()),
        )
        .with_writer(std::io::stderr)
        .without_time()
        .init();
}

async fn run(cli: Cli) -> Result<u8> {
    let config_path = match &cli.config {
        Some(p) => p.clone(),
        None => Config::default_path().context("resolving the config location")?,
    };

    let mut config =
        Config::load(&config_path).with_context(|| format!("loading {}", config_path.display()))?;

    // The policy engine needs to know our privilege level to apply the
    // elevation rules; it is a runtime fact, never persisted.
    config.policy.elevated = platform::is_elevated();
    odysync_backends::direct::http::set_proxy(config.proxy_url.as_deref());

    let style = Style::detect();

    match cli.command {
        Command::Backends => {
            let backends = odysync_backends::detect_backends(&config).await;
            if cli.json {
                let list: Vec<_> = backends
                    .iter()
                    .map(|b| {
                        serde_json::json!({
                            "id": b.kind().id(),
                            "name": b.display_name(),
                            "verification": odysync_core::verification_of(b.kind()),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else if backends.is_empty() {
                println!("No supported package managers were found on this system.");
            } else {
                println!("{}", style.bold("Detected package managers\n"));
                for b in &backends {
                    // Naming who verifies the payload, rather than showing a
                    // tick that could be read as "Odysync checked this". It
                    // does not: the payload is downloaded and validated inside
                    // the package manager's own process.
                    let verification = odysync_core::verification_of(b.kind());
                    println!(
                        "  {:<24}  {:<38}  {}",
                        b.kind().id(),
                        b.display_name(),
                        style.dim(&verification.label()),
                    );
                }
                println!(
                    "\n{}",
                    style.dim(&format!(
                        "Running on {} {}elevated",
                        platform::os_label(),
                        if config.policy.elevated { "" } else { "un" }
                    ))
                );
            }
            Ok(0)
        }

        Command::Scan {
            show_skipped,
            backend,
        } => {
            let backends = select_backends(&config, &backend).await?;
            let scan = scan_all(&config, &backends).await;
            let plan = config.policy.plan(scan.candidates);

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            } else {
                print!("{}", render::plan_table(&plan, &style, show_skipped));
            }
            // Nothing could be asked: whatever calls this must not read the
            // empty list as "up to date".
            Ok(if scan.nothing_answered { 2 } else { 0 })
        }

        Command::Apply {
            yes,
            dry_run,
            only,
            backend,
            profile,
            report: report_path,
            report_format,
            restore_point,
            unattended,
        } => {
            let backends = select_backends(&config, &backend).await?;
            let scan = scan_all(&config, &backends).await;
            let nothing_answered = scan.nothing_answered;
            let mut candidates = scan.candidates;
            if unattended {
                candidates.retain(|c| c.id.backend.in_unattended_care());
            }

            // Narrow before planning, so the summary reflects what the user
            // actually asked for rather than the whole machine.
            let filters = resolve_filters(&config, &only, profile.as_deref())?;
            if let Some(filters) = &filters {
                candidates.retain(|c| matches_any(c, filters));
            }

            let plan = config.policy.plan(candidates);
            let actionable = plan.iter().filter(|p| p.is_actionable()).count();

            if !cli.json {
                print!("{}", render::plan_table(&plan, &style, false));
            }

            if actionable == 0 {
                if cli.json {
                    // The same shape as after a run, so a script need not
                    // treat "nothing to do" as a special case.
                    let mut report = RunReport::new();
                    for p in &plan {
                        if let Some(reason) = &p.blocked_by {
                            report.push(
                                p.candidate.id.clone(),
                                p.candidate.name.clone(),
                                odysync_core::ApplyOutcome::Skipped {
                                    reason: reason.clone(),
                                },
                            );
                        }
                    }
                    report.finish();
                    println!("{}", serde_json::to_string_pretty(&report)?);
                }
                return Ok(if nothing_answered { 2 } else { 0 });
            }

            #[cfg(windows)]
            let driver_updates = plan
                .iter()
                .filter(|p| p.is_actionable())
                .filter(|p| {
                    p.candidate.id.backend == odysync_core::model::BackendKind::WindowsDrivers
                })
                .count();

            // A driver update is the one routine change that can leave a
            // machine without a display or a network card. Say so before the
            // confirmation prompt, not after.
            #[cfg(windows)]
            if driver_updates > 0 && !dry_run && !cli.json {
                driver_backup_notice(&config, driver_updates, &style).await;
            }

            if !yes && !dry_run && !confirm(actionable)? {
                eprintln!("Cancelled.");
                return Ok(0);
            }

            let restore = restore_point || config.restore_point;
            let report = run_updates(&config, &backends, &plan, dry_run, restore).await?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", render::summary(&report, &style));
                if dry_run {
                    println!("{}", style.dim("Dry run: nothing was installed."));
                }
            }

            if let Some(path) = report_path {
                let text = match report_format {
                    ReportFormat::Json => serde_json::to_string_pretty(&report)?,
                    ReportFormat::Text => report.to_text(),
                };
                std::fs::write(&path, text)
                    .with_context(|| format!("writing report to {}", path.display()))?;
            }

            Ok(report.exit_code() as u8)
        }

        Command::Hold { package, pin, note } => {
            config
                .policy
                .holds
                .retain(|h| !h.package.eq_ignore_ascii_case(&package));
            config.policy.holds.push(odysync_core::policy::Hold {
                package: package.clone(),
                pin: pin.clone(),
                note,
            });
            // `elevated` is runtime-only and marked `#[serde(skip)]`, so it is
            // not written back to disk here.
            config.save(&config_path)?;
            match pin {
                Some(v) => println!("Pinned {package} to {v}."),
                None => println!("Held {package}; it will not be updated."),
            }
            Ok(0)
        }

        Command::Unhold { package } => {
            let before = config.policy.holds.len();
            config
                .policy
                .holds
                .retain(|h| !h.package.eq_ignore_ascii_case(&package));
            if config.policy.holds.len() == before {
                println!("{package} was not held.");
            } else {
                config.save(&config_path)?;
                println!("Released {package}.");
            }
            Ok(0)
        }

        Command::Pause {
            backend,
            days,
            note,
        } => {
            let Some(kind) = odysync_core::model::BackendKind::from_id(&backend) else {
                anyhow::bail!(
                    "unknown backend {backend:?}. `odysync backends` lists the ones on this machine."
                );
            };

            let until =
                jiff::Timestamp::now() + jiff::SignedDuration::from_hours(24 * i64::from(days));
            config.pause_backend(kind, until, note);
            config.save(&config_path)?;

            println!(
                "Paused {} for {days} day(s), until {}.",
                kind.id(),
                until.strftime("%Y-%m-%d %H:%M UTC")
            );
            // Said out loud because a paused security backend is exactly the
            // state a user can forget they put the machine into.
            if kind.requires_elevation() {
                println!(
                    "{}",
                    style.dim("  It resumes on its own; `odysync resume` lifts it sooner.")
                );
            }
            Ok(0)
        }

        Command::Resume { backend } => {
            let Some(kind) = odysync_core::model::BackendKind::from_id(&backend) else {
                anyhow::bail!(
                    "unknown backend {backend:?}. `odysync backends` lists the ones on this machine."
                );
            };

            if config.resume_backend(kind) {
                config.save(&config_path)?;
                println!("Resumed {}.", kind.id());
            } else {
                println!("{} was not paused.", kind.id());
            }
            Ok(0)
        }

        Command::SelfCheck => {
            let check = odysync_backends::self_update::check(config.proxy_url.as_deref()).await?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&check)?);
                return Ok(0);
            }

            match (&check.latest, check.is_newer) {
                (Some(latest), true) => {
                    println!(
                        "{} {} is available; you are running {}.",
                        style.bold("Update:"),
                        latest,
                        check.current
                    );
                    if let Some(url) = &check.release_url {
                        println!("  {url}");
                    }
                    if let Some(sha) = &check.installer_sha256 {
                        println!("  checksum: {sha}");
                    }
                    if let Some(reason) = &check.not_installable_reason {
                        println!("\n{}", style.dim(reason));
                    }
                }
                (Some(latest), false) => println!(
                    "Up to date: running {}, latest published is {latest}.",
                    check.current
                ),
                (None, _) => println!("Could not determine the latest release."),
            }
            Ok(0)
        }

        Command::Config => {
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&config)?);
            } else {
                println!("{} {}", style.bold("Config file:"), config_path.display());
                println!(
                    "{}",
                    style.dim(if config_path.exists() {
                        ""
                    } else {
                        "(not created yet; defaults are in use)"
                    })
                );
                println!("\n{}", serde_json::to_string_pretty(&config)?);
            }
            Ok(0)
        }

        Command::Maintain { action } => {
            let kind: MaintenanceKind = action.into();
            if cli.json {
                let result = odysync_backends::maintenance::run_maintenance(kind)
                    .await
                    .context("running maintenance action")?;
                println!("{}", serde_json::to_string_pretty(&result)?);
                return Ok(u8::from(!result.success));
            } else {
                println!("{}", style.bold(&format!("Running: {kind}\n")));
                let result = odysync_backends::maintenance::run_maintenance(kind)
                    .await
                    .context("running maintenance action")?;
                if result.success {
                    println!("  {} {}", style.green("ok"), result.summary);
                } else {
                    println!("  {} {}", style.red("!!"), result.summary);
                    return Ok(1);
                }
            }
            Ok(0)
        }

        Command::Schedule {
            frequency,
            time,
            task_name,
            extra_args,
            care,
        } => {
            use odysync_backends::scheduler::{ScheduleSpec, ScheduledCommand};
            let spec = ScheduleSpec {
                frequency: frequency.into(),
                time,
                task_name: task_name.clone(),
                extra_args,
                program: None,
                command: if care {
                    ScheduledCommand::Care
                } else {
                    ScheduledCommand::Apply
                },
            };
            odysync_backends::scheduler::create_schedule(&spec)
                .await
                .context("creating scheduled task")?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "scheduled": true,
                        "task_name": task_name,
                        "frequency": spec.frequency.to_string(),
                        "time": spec.time,
                    }))?
                );
            } else {
                println!(
                    "Scheduled {} {} at {}.",
                    spec.frequency, task_name, spec.time
                );
            }
            Ok(0)
        }

        Command::Unschedule { task_name } => {
            let existed = odysync_backends::scheduler::schedule_exists(&task_name).await;
            if existed {
                odysync_backends::scheduler::remove_schedule(&task_name)
                    .await
                    .context("removing the scheduled task")?;
            }
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "removed": existed,
                        "task_name": task_name,
                    }))?
                );
            } else if existed {
                println!("Removed scheduled task: {task_name}");
            } else {
                println!("No scheduled task named '{task_name}' was found.");
            }
            Ok(0)
        }

        Command::Diagnostics { out } => {
            odysync_backends::diagnostics::create_diagnostics(&out, &config, None)
                .await
                .with_context(|| format!("creating diagnostics bundle at {}", out.display()))?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "path": out.display().to_string(),
                    }))?
                );
            } else {
                println!("Diagnostics bundle written: {}", out.display());
            }
            Ok(0)
        }

        Command::Daemon {
            interval,
            apply,
            restore_point,
            once,
            on_battery,
        } => {
            let opts = daemon::DaemonOpts {
                interval_minutes: interval,
                auto_apply: apply,
                restore_point,
                once,
                on_battery,
            };
            let code = daemon::run(&opts, &config_path).await?;
            Ok(code)
        }

        #[cfg(windows)]
        Command::Drivers { action } => run_driver_action(action, cli.json, &style).await,

        Command::Clean { apply, only, yes } => run_clean(apply, only, yes, cli.json, &style).await,

        Command::Care { yes, dry_run } => run_care(&config, yes, dry_run, cli.json, &style).await,

        Command::Health => {
            let report = odysync_backends::device_health::report().await;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", render::health(&report, &style));
            }
            Ok(0)
        }

        Command::Components { action } => match action {
            None => {
                let list = odysync_backends::components::status().await;
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&list)?);
                } else {
                    print!("{}", render::components(&list, &style));
                }
                Ok(0)
            }
            Some(ComponentAction::Install { id }) => {
                let _lock = odysync_core::ApplyLock::acquire()?;
                match odysync_backends::components::install(&id).await {
                    Ok(message) => {
                        println!("{message}");
                        Ok(0)
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                        Ok(1)
                    }
                }
            }
        },

        Command::Startup { action } => match action {
            None => {
                let items = odysync_backends::startup_items::list().await?;
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&items)?);
                } else if items.is_empty() {
                    println!("Nothing starts automatically.");
                } else {
                    println!("{}", style.bold("Starts when you sign in\n"));
                    for i in &items {
                        let state = if i.enabled { "on " } else { "off" };
                        println!(
                            "  {}  {:<32} {}",
                            if i.enabled {
                                style.bold(state)
                            } else {
                                style.dim(state)
                            },
                            i.label.as_deref().unwrap_or(&i.name),
                            style.dim(&format!("{}  [{}]", i.name, i.location))
                        );
                    }
                }
                Ok(0)
            }
            Some(StartupAction::Enable { name, location }) => {
                odysync_backends::startup_items::set_enabled(&name, &location, true).await?;
                println!("{name} will start when you sign in.");
                Ok(0)
            }
            Some(StartupAction::Disable { name, location }) => {
                odysync_backends::startup_items::set_enabled(&name, &location, false).await?;
                println!(
                    "{name} no longer starts when you sign in. `odysync startup enable` undoes this."
                );
                Ok(0)
            }
        },

        Command::Sysinfo => {
            let r = odysync_backends::system_info::report().await;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&r)?);
            } else {
                println!("{}", style.bold("This machine\n"));
                println!("  {:<10} {}", "System", r.os);
                if let Some(m) = &r.machine {
                    println!("  {:<10} {m}", "Model");
                }
                println!(
                    "  {:<10} {} ({} threads)",
                    "Processor", r.cpu, r.cpu_threads
                );
                println!("  {:<10} {}", "Memory", render::human_bytes(r.memory_bytes));
                for g in &r.gpus {
                    println!(
                        "  {:<10} {}{}",
                        "Graphics",
                        g.name,
                        g.driver_version
                            .as_deref()
                            .map(|v| style.dim(&format!("  driver {v}")))
                            .unwrap_or_default()
                    );
                }
                for d in &r.disks {
                    println!(
                        "  {:<10} {}  {} free of {}  {}",
                        "Disk",
                        d.mount,
                        render::human_bytes(d.free_bytes),
                        render::human_bytes(d.size_bytes),
                        style.dim(&d.filesystem)
                    );
                }
            }
            Ok(0)
        }

        Command::Devices => {
            let problems = odysync_backends::devices::problems().await;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&problems)?);
            } else if problems.is_empty() {
                println!("Every device is working.");
            } else {
                println!("{}", style.bold("Devices with problems\n"));
                for d in &problems {
                    println!(
                        "  {}  {}",
                        style.bold(&d.name),
                        style.dim(&format!("({}, code {})", d.class, d.code))
                    );
                    println!("    {}", d.meaning);
                    println!("    {}\n", style.dim(&d.suggestion));
                }
            }
            Ok(0)
        }

        Command::LargeFiles { limit, min_mb } => {
            let report = odysync_backends::cleanup::largest_files(
                limit,
                min_mb.saturating_mul(1024 * 1024),
                std::time::Duration::from_secs(120),
            )
            .await;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", render::large_files(&report, &style));
            }
            Ok(0)
        }

        #[cfg(windows)]
        Command::OptimizeDisk { apply } => run_optimize_disk(apply, cli.json, &style).await,
    }
}

/// `odysync care`: the updates that pass every rule, then the safe cleanup.
///
/// Deliberately narrower than `apply` followed by `clean --apply`: no filters,
/// no drivers, firmware, OS upgrades or developer packages (pip, npm, cargo …),
/// no opt-in or destructive cleanup
/// categories, and the restore point setting from the config. It is what a
/// schedule runs while nobody is watching.
async fn run_care(
    config: &Config,
    yes: bool,
    dry_run: bool,
    json: bool,
    style: &Style,
) -> Result<u8> {
    use odysync_core::cleanup::CleanupCategory;

    // ── Look ──
    let backends = select_backends(config, &[]).await?;
    let scan = scan_all(config, &backends).await;
    let nothing_answered = scan.nothing_answered;
    let candidates = scan.candidates;
    // Drivers, firmware, OS upgrades and developer packages wait for a
    // person; see `BackendKind::in_unattended_care`.
    let (attended, plan): (Vec<_>, Vec<_>) = config
        .policy
        .plan(candidates)
        .into_iter()
        .partition(|p| p.is_actionable() && !p.candidate.id.backend.in_unattended_care());
    let actionable = plan.iter().filter(|p| p.is_actionable()).count();

    let findings = odysync_backends::cleanup::scan().await;
    let to_clean: Vec<CleanupCategory> = findings
        .iter()
        .filter(|f| f.is_actionable() && f.category.selected_by_default())
        .filter(|f| !f.category.reversibility().needs_explicit_confirmation())
        .map(|f| f.category)
        .collect();
    let reclaimable: u64 = findings
        .iter()
        .filter(|f| to_clean.contains(&f.category))
        .map(|f| f.bytes)
        .sum();

    if !json {
        print!("{}", render::plan_table(&plan, style, false));
        println!(
            "{} {} in {} cleanup categor{} that are safe to clear.",
            style.bold("Cleanup:"),
            render::human_bytes(reclaimable),
            to_clean.len(),
            if to_clean.len() == 1 { "y" } else { "ies" }
        );
        if !attended.is_empty() {
            println!(
                "{}",
                style.dim(&format!(
                    "{} driver, firmware, system or developer-package update(s) are left for you \
                     to start with `odysync apply`.",
                    attended.len()
                ))
            );
        }
    }

    if actionable == 0 && to_clean.is_empty() {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "updates": serde_json::Value::Null,
                    "cleanup": [],
                    "reclaimed_bytes": 0,
                }))?
            );
        } else if nothing_answered {
            println!("\nNo update source could be reached, so there may still be updates.");
        } else {
            println!("\nNothing to do: this machine is up to date and tidy.");
        }
        return Ok(if nothing_answered { 2 } else { 0 });
    }

    if dry_run {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "dry_run": true,
                    "updates": plan,
                    "cleanup": to_clean.iter().map(|c| c.id()).collect::<Vec<_>>(),
                    "reclaimable_bytes": reclaimable,
                    "left_for_you": attended,
                }))?
            );
        } else {
            println!(
                "\n{}",
                style.dim("Dry run: nothing was installed or deleted.")
            );
        }
        return Ok(0);
    }

    if !yes {
        use std::io::BufRead;
        eprint!(
            "\nInstall {actionable} update(s) and clear {}? [y/N] ",
            render::human_bytes(reclaimable)
        );
        let mut line = String::new();
        // A closed stdin is never consent.
        if std::io::stdin().lock().read_line(&mut line)? == 0
            || !matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
        {
            println!("Cancelled.");
            return Ok(0);
        }
    }

    // ── Updates ──
    let report = if actionable > 0 {
        run_updates(config, &backends, &plan, false, config.restore_point).await?
    } else {
        RunReport::new()
    };

    // ── Cleanup ──
    let results = if to_clean.is_empty() {
        Vec::new()
    } else {
        odysync_backends::cleanup::clean(to_clean).await
    };
    let reclaimed: u64 = results.iter().map(|(_, o)| o.removed_bytes).sum();

    if json {
        let cleanup: Vec<_> = results
            .iter()
            .map(|(c, o)| serde_json::json!({ "category": c.id(), "outcome": o }))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "updates": report,
                "cleanup": cleanup,
                "reclaimed_bytes": reclaimed,
            }))?
        );
    } else {
        if actionable > 0 {
            print!("{}", render::summary(&report, style));
        }
        if !results.is_empty() {
            println!(
                "{} {}",
                style.bold("Cleared:"),
                render::human_bytes(reclaimed)
            );
        }
    }

    Ok(report.exit_code() as u8)
}

/// `odysync clean`.
async fn run_clean(
    apply: bool,
    only: Vec<String>,
    yes: bool,
    json: bool,
    style: &Style,
) -> Result<u8> {
    use odysync_core::cleanup::{CleanupCategory, default_reclaimable};

    let findings = odysync_backends::cleanup::scan().await;

    // One document per run: the findings, or after `--apply` the results.
    if json && !apply {
        println!("{}", serde_json::to_string_pretty(&findings)?);
        return Ok(0);
    }

    if !json {
        println!("{}", style.bold("Reclaimable disk space\n"));
        for f in &findings {
            let size = match (&f.error, f.bytes, f.file_count) {
                (Some(e), _, _) => format!("could not measure: {e}"),
                (None, 0, 0) => "nothing found".to_string(),
                (None, b, n) => format!("{} in {n} files", render::human_bytes(b)),
            };
            let marker = if f.category.is_report_only() {
                "report only"
            } else if f.category.selected_by_default() {
                "default"
            } else {
                "opt-in"
            };
            println!(
                "  {:<22} {:>22}   {}",
                f.category.id(),
                size,
                style.dim(marker)
            );
            if let Some(note) = &f.note {
                println!("  {:<22} {}", "", style.dim(note));
            }
        }

        println!(
            "\n{} {} in the categories that are safe by default.",
            style.bold("Total:"),
            render::human_bytes(default_reclaimable(&findings)),
        );
    }

    if !apply {
        if !json {
            println!(
                "\n{}",
                style.dim("Nothing was deleted. Add --apply to reclaim it.")
            );
        }
        return Ok(0);
    }

    // Resolve which categories to act on. An unknown id is an error rather
    // than a silent no-op: a typo must not read as "cleaned successfully".
    let selected: Vec<CleanupCategory> = if only.is_empty() {
        findings
            .iter()
            .filter(|f| f.is_actionable() && f.category.selected_by_default())
            .map(|f| f.category)
            .collect()
    } else {
        let mut chosen = Vec::new();
        for id in &only {
            let Some(category) = CleanupCategory::from_id(id) else {
                anyhow::bail!(
                    "unknown cleanup category {id:?}. Run `odysync clean` to see the ids."
                );
            };
            chosen.push(category);
        }
        chosen
    };

    if selected.is_empty() {
        println!("{}", if json { "[]" } else { "\nNothing to clean." });
        return Ok(0);
    }

    // Anything that cannot be undone is named individually before the prompt,
    // not folded into a count.
    let destructive: Vec<&CleanupCategory> = selected
        .iter()
        .filter(|c| c.reversibility().needs_explicit_confirmation())
        .collect();
    if !destructive.is_empty() {
        eprintln!("\n{}", style.bold("This cannot be undone:"));
        for c in &destructive {
            eprintln!("  {}: {}", c.label(), c.consequence());
        }
    }

    if !yes && !confirm_clean(selected.len())? {
        eprintln!("Cancelled.");
        return Ok(0);
    }

    let results = odysync_backends::cleanup::clean(selected).await;

    if json {
        let payload: Vec<_> = results
            .iter()
            .map(|(c, o)| serde_json::json!({ "category": c.id(), "outcome": o }))
            .collect();
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(0);
    }

    let mut total = 0u64;
    println!();
    for (category, outcome) in &results {
        total += outcome.removed_bytes;
        println!(
            "  {:<22} {} in {} files",
            category.id(),
            render::human_bytes(outcome.removed_bytes),
            outcome.removed_files
        );
        // Locked files are the normal case on Windows, so these are listed
        // rather than treated as a failure.
        for skipped in outcome.skipped.iter().take(5) {
            println!("  {:<22} {}", "", style.dim(skipped));
        }
        if outcome.skipped.len() > 5 {
            println!(
                "  {:<22} {}",
                "",
                style.dim(&format!("... and {} more", outcome.skipped.len() - 5))
            );
        }
    }
    println!(
        "\n{} {}",
        style.bold("Reclaimed:"),
        render::human_bytes(total)
    );
    Ok(0)
}

/// Ask before deleting. A closed stdin is never consent.
fn confirm_clean(count: usize) -> Result<bool> {
    use std::io::BufRead;

    eprint!(
        "\nClean {count} categor{}? [y/N] ",
        if count == 1 { "y" } else { "ies" }
    );

    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line)? == 0 {
        return Ok(false);
    }
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// `odysync optimize-disk`.
#[cfg(windows)]
async fn run_optimize_disk(apply: bool, json: bool, style: &Style) -> Result<u8> {
    let plan = odysync_backends::cleanup::plan_disk_optimization().await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&plan)?);
    } else if plan.is_empty() {
        println!("No volumes with a drive letter were found.");
    } else {
        println!("{}", style.bold("Disk optimisation plan\n"));
        for v in &plan {
            println!("  {:<6} {:<10} {}", v.drive, v.media_type, v.operation);
        }
        // The media type decides the operation and there is no override:
        // defragmenting an SSD spends its erase cycles for no benefit.
        println!(
            "\n{}",
            style.dim(
                "SSDs are trimmed, never defragmented. The media type decides; \
                 there is no override."
            )
        );
    }

    if !apply {
        if !json && !plan.is_empty() {
            println!("{}", style.dim("Add --apply to run it."));
        }
        return Ok(0);
    }

    let report = odysync_backends::cleanup::run_disk_optimization(&plan).await;
    for line in &report {
        println!("  {line}");
    }
    Ok(0)
}

/// Take a driver backup before applying, or say plainly that there is none.
///
/// Backing up automatically is off by default because a full driver-store
/// export is a copy of `DriverStore\FileRepository` — 4.7 GB on the machine
/// this was measured on. Silently spending that, twice over, on the disk of
/// the machine we are maintaining is not a good default. Silently *not* having
/// a rollback is not acceptable either, so the user is told which of the two
/// they are getting.
#[cfg(windows)]
async fn driver_backup_notice(config: &Config, driver_updates: usize, style: &Style) {
    use odysync_backends::driver_backup;

    if config.driver_backup_before_apply {
        println!(
            "\nExporting the driver store before applying {driver_updates} driver update(s). \
             This copies several gigabytes and takes a few minutes."
        );
        match driver_backup::create_backup(&format!(
            "before applying {driver_updates} driver update(s)"
        ))
        .await
        {
            Ok(manifest) => {
                println!(
                    "Backed up {} package(s) as {}.",
                    manifest.packages.len(),
                    manifest.id
                );
                if let Err(e) = driver_backup::prune(config.driver_backup_keep as usize) {
                    tracing::warn!(error = %e, "could not prune old driver backups");
                }
            }
            // A failed backup must not silently become "no backup". It also
            // must not block an update the user asked for; they are told and
            // then asked to confirm, which is the next thing that happens.
            Err(e) => eprintln!("warning: the driver backup failed: {e}"),
        }
        return;
    }

    let existing = driver_backup::list_backups().unwrap_or_default();
    match existing.first() {
        Some(latest) => println!(
            "\n{} {} driver update(s). Most recent driver backup: {} ({}).",
            style.bold("Note:"),
            driver_updates,
            latest.id,
            latest.reason
        ),
        None => println!(
            "\n{} applying {} driver update(s) with no driver backup on disk.\n  \
             Take one first with `odysync drivers backup`, or set \
             `driver-backup-before-apply` in the config to do it automatically.\n  \
             Windows keeps its own single-step rollback in Device Manager either way.",
            style.bold("Note:"),
            driver_updates,
        ),
    }
}

/// `odysync drivers <action>`.
#[cfg(windows)]
async fn run_driver_action(action: DriverAction, json: bool, style: &Style) -> Result<u8> {
    use odysync_backends::driver_backup;

    match action {
        DriverAction::Backup { reason, only } => {
            let mut packages = driver_backup::list_packages().await?;
            if packages.is_empty() {
                println!("No third-party driver packages are installed; nothing to back up.");
                return Ok(0);
            }

            if !only.is_empty() {
                let wanted: Vec<String> =
                    only.iter().map(|s| s.trim().to_ascii_lowercase()).collect();
                packages.retain(|p| wanted.contains(&p.published_name.to_ascii_lowercase()));

                // A name that matched nothing is an error, not an empty
                // backup: silently exporting zero packages would look like a
                // successful backup right up until someone needed it.
                for name in &wanted {
                    if !packages
                        .iter()
                        .any(|p| p.published_name.eq_ignore_ascii_case(name))
                    {
                        anyhow::bail!(
                            "no driver package named {name:?} is installed. \
                             Published names look like `oem23.inf`."
                        );
                    }
                }
            }

            println!(
                "Exporting {} driver package(s). This copies the whole driver store and \
                 can take a few minutes.",
                packages.len()
            );
            let manifest = driver_backup::create_backup_of(&reason, &packages).await?;

            if json {
                println!("{}", serde_json::to_string_pretty(&manifest)?);
                return Ok(0);
            }

            println!(
                "\n{} {} package(s), {:.1} MB, id {}",
                style.bold("Backed up"),
                manifest.packages.len(),
                manifest.total_size_bytes() as f64 / (1024.0 * 1024.0),
                manifest.id,
            );
            // A partial backup restores less than the user thinks it will, so
            // it is called out rather than folded into the success line.
            if !manifest.is_complete() {
                eprintln!(
                    "\nwarning: {} package(s) could not be exported:",
                    manifest.failures.len()
                );
                for failure in &manifest.failures {
                    eprintln!("  {failure}");
                }
            }
            Ok(0)
        }

        DriverAction::List => {
            let backups = driver_backup::list_backups()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&backups)?);
                return Ok(0);
            }
            if backups.is_empty() {
                println!("No driver backups yet. Create one with `odysync drivers backup`.");
                return Ok(0);
            }

            println!("{}", style.bold("Driver backups (newest first)\n"));
            for b in &backups {
                println!(
                    "  {:<22}  {:>4} pkgs  {:>8.1} MB  {}{}",
                    b.id,
                    b.packages.len(),
                    b.total_size_bytes() as f64 / (1024.0 * 1024.0),
                    b.reason,
                    if b.is_complete() {
                        String::new()
                    } else {
                        format!("  ({} failed)", b.failures.len())
                    },
                );
            }
            Ok(0)
        }

        DriverAction::Rollback { to } => {
            let _lock = odysync_core::ApplyLock::acquire()?;
            let report = driver_backup::restore(&to).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
                return Ok(0);
            }

            println!(
                "Re-added {} package(s) onto a device; {} already current; {} failed.",
                report.restored.len(),
                report.already_current.len(),
                report.failed.len()
            );
            for name in &report.already_current {
                println!(
                    "  {}",
                    style.dim(&format!(
                        "{name}: back in the driver store, but no device needed it"
                    ))
                );
            }
            for failure in &report.failed {
                eprintln!("  {failure}");
            }
            if report.reboot_required {
                println!("\n{}", style.bold("A restart is required to finish."));
            }
            // Said every time, because it is the part people get wrong: this
            // puts the old package back, it does not remove the new one.
            println!(
                "\n{}",
                style.dim(
                    "Windows ranks driver packages. If a newer package is still in the \
                     store it may keep being preferred; remove it from Device Manager to \
                     force the older one."
                )
            );
            Ok(if report.is_success() { 0 } else { 1 })
        }

        DriverAction::Prune { keep } => {
            let removed = driver_backup::prune(keep)?;
            println!("Removed {removed} old driver backup(s), keeping {keep}.");
            Ok(0)
        }
    }
}

/// What a scan of several sources came to.
pub(crate) struct Scan {
    pub candidates: Vec<UpdateCandidate>,
    /// Sources were asked and not one of them answered.
    pub nothing_answered: bool,
}

/// Scan every backend concurrently.
///
/// A backend that errors or takes too long is reported and contributes nothing
/// rather than failing the whole run: one broken package manager must not hide
/// updates available from the others.
pub(crate) async fn scan_all(config: &Config, backends: &[Box<dyn Backend>]) -> Scan {
    use odysync_backends::scan::{ScanCancel, scan_sources};

    let refs: Vec<&dyn Backend> = backends.iter().map(|b| b.as_ref()).collect();
    let limit = std::time::Duration::from_secs(u64::from(config.backend_timeout_secs.max(10)));
    let progress = Progress::new();
    let results = scan_sources(&refs, limit, &ScanCancel::new(), &|p| progress.scan(&p)).await;
    progress.clear();

    let nothing_answered = !results.is_empty() && results.iter().all(|r| r.result.is_err());
    let mut candidates = Vec::new();
    for source in results {
        match source.result {
            Ok(found) => candidates.extend(found),
            Err(e) => eprintln!("warning: {} could not be checked: {e}", source.name),
        }
    }
    Scan {
        candidates,
        nothing_answered,
    }
}

/// Install `plan`: one run at a time on this machine, recorded in the history,
/// stopping after the current package on Ctrl-C.
pub(crate) async fn run_updates(
    config: &Config,
    backends: &[Box<dyn Backend>],
    plan: &[odysync_core::PlannedUpdate],
    dry_run: bool,
    restore_point: bool,
) -> Result<RunReport> {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    let _lock = if dry_run {
        None
    } else {
        Some(odysync_core::ApplyLock::acquire()?)
    };

    let cancel = Arc::new(AtomicBool::new(false));
    let _installing = interrupt::Installing::begin(Arc::clone(&cancel));

    let refs: Vec<&dyn Backend> = backends.iter().map(|b| b.as_ref()).collect();
    let mut runner = Runner::new(refs, dry_run)
        .with_max_retries(config.max_retries)
        .with_cancel(cancel);
    if !dry_run {
        runner = runner.with_history(odysync_core::UpdateHistory::load());
    }
    let mut report = RunReport::new();
    let progress = Progress::new();
    runner
        .run_with_progress(plan, &mut report, restore_point, Some(&progress))
        .await;
    progress.clear();
    Ok(report)
}

/// What Ctrl-C does.
///
/// Once anything in a process has asked for the signal, its default (end the
/// process) is gone for good, so a listener that comes and goes with each
/// install would leave a daemon that can no longer be stopped. There is one
/// listener, and it ends the process whenever nothing is being installed.
mod interrupt {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, Once};

    static CURRENT: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);
    static LISTENER: Once = Once::new();

    /// While this lives, Ctrl-C stops after the current package.
    pub struct Installing(());

    impl Installing {
        pub fn begin(cancel: Arc<AtomicBool>) -> Self {
            *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(cancel);
            LISTENER.call_once(|| {
                tokio::spawn(async {
                    while tokio::signal::ctrl_c().await.is_ok() {
                        if should_exit() {
                            std::process::exit(130);
                        }
                        eprintln!(
                            "\nStopping after the package being installed. Press Ctrl-C again \
                             to quit now."
                        );
                    }
                });
            });
            Self(())
        }
    }

    impl Drop for Installing {
        fn drop(&mut self) {
            *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }

    /// Nothing is being installed, or it was asked for twice.
    pub fn should_exit() -> bool {
        match &*CURRENT.lock().unwrap_or_else(|e| e.into_inner()) {
            None => true,
            Some(cancel) => cancel.swap(true, Ordering::Relaxed),
        }
    }
}

/// One line on the terminal that says what is happening.
///
/// Goes to stderr, and only when that is a terminal, so neither `--json` nor a
/// log file sees it.
struct Progress {
    enabled: bool,
    waiting: std::sync::Mutex<Vec<String>>,
}

impl Progress {
    fn new() -> Self {
        use std::io::IsTerminal;
        Self {
            enabled: std::io::stderr().is_terminal(),
            waiting: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn line(&self, text: &str) {
        if self.enabled {
            let text: String = text.chars().take(78).collect();
            eprint!("\r{text:<79}\r");
        }
    }

    fn clear(&self) {
        self.line("");
    }

    fn scan(&self, p: &odysync_backends::scan::ScanProgress) {
        use odysync_backends::scan::SourceState;
        let mut waiting = self.waiting.lock().unwrap_or_else(|e| e.into_inner());
        match p.state {
            SourceState::Started => waiting.push(p.name.clone()),
            SourceState::Done | SourceState::Failed => waiting.retain(|n| *n != p.name),
        }
        self.line(&scan_line(p.done, p.total, &waiting));
    }
}

impl odysync_core::ProgressEmitter for Progress {
    fn emit_progress(&self, event: odysync_core::ProgressEvent) {
        if event.phase != "done" {
            self.line(&format!(
                "[{}/{}] {}",
                (event.current + 1).min(event.total),
                event.total,
                event.package
            ));
        }
    }
}

/// `Checked 12 of 21 sources, waiting for Python pip, Windows Update ...`
fn scan_line(done: usize, total: usize, waiting: &[String]) -> String {
    let mut text = format!("Checked {done} of {total} sources");
    if !waiting.is_empty() && done > 0 {
        let names: Vec<&str> = waiting.iter().take(2).map(String::as_str).collect();
        text.push_str(", waiting for ");
        text.push_str(&names.join(", "));
        if waiting.len() > 2 {
            text.push_str(" ...");
        }
    }
    text
}

/// The backends a scan or apply should use.
///
/// With no `--backend` flags this is everything enabled and present. Naming a
/// backend narrows the run to it — and includes it even when it is opt-in,
/// because typing its name is the opt-in. A backend the user disabled in the
/// config stays disabled; a flag is not a reason to override that.
async fn select_backends(config: &Config, named: &[String]) -> Result<Vec<Box<dyn Backend>>> {
    if named.is_empty() {
        return Ok(odysync_backends::detect_backends(config).await);
    }
    for id in named {
        if odysync_core::model::BackendKind::from_id(id).is_none() {
            anyhow::bail!("unknown backend '{id}'; `odysync backends` lists them");
        }
    }
    let wanted = |kind: odysync_core::model::BackendKind| {
        named
            .iter()
            .any(|n| n.trim().eq_ignore_ascii_case(kind.id()))
    };
    Ok(odysync_backends::probe_kinds(config, &wanted)
        .await
        .into_iter()
        .filter(|p| p.available)
        .filter(|p| {
            !config
                .disabled_backends
                .iter()
                .any(|d| d.trim().eq_ignore_ascii_case(p.backend.kind().id()))
        })
        .map(|p| p.backend)
        .collect())
}

/// Work out which packages the user restricted the run to, if any.
fn resolve_filters(
    config: &Config,
    only: &[String],
    profile: Option<&str>,
) -> Result<Option<Vec<String>>> {
    let mut filters: Vec<String> = only.to_vec();

    if let Some(name) = profile {
        let p = config
            .profile(name)
            .with_context(|| format!("no profile named '{name}' in the config"))?;
        filters.extend(p.packages.iter().cloned());
    }

    Ok(if filters.is_empty() {
        None
    } else {
        Some(filters)
    })
}

/// Does this candidate match any of `filters` (by bare id or `backend:id`)?
fn matches_any(candidate: &UpdateCandidate, filters: &[String]) -> bool {
    filters.iter().any(|f| {
        let f = f.trim();
        f.eq_ignore_ascii_case(&candidate.id.native)
            || f.eq_ignore_ascii_case(&candidate.id.to_string())
            || candidate
                .id
                .alias
                .as_deref()
                .is_some_and(|a| f.eq_ignore_ascii_case(a))
    })
}

/// Ask before changing the system.
fn confirm(count: usize) -> Result<bool> {
    use std::io::BufRead;

    // On stderr, so that `--json` on stdout stays a document.
    eprint!("\nInstall {count} update(s)? [y/N] ");

    let mut line = String::new();
    // A closed stdin (a pipe, a service) must not be read as consent.
    if std::io::stdin().lock().read_line(&mut line)? == 0 {
        return Ok(false);
    }
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_is_consistent() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn ctrl_c_ends_the_process_unless_something_is_being_installed() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            assert!(interrupt::should_exit());
            let flag = Arc::new(AtomicBool::new(false));
            let installing = interrupt::Installing::begin(Arc::clone(&flag));
            assert!(!interrupt::should_exit(), "the first one only asks");
            assert!(flag.load(Ordering::Relaxed));
            assert!(interrupt::should_exit(), "the second one means it");
            drop(installing);
            assert!(interrupt::should_exit());
        });
    }

    /// What the scheduler registers must be something the command accepts.
    #[test]
    fn a_scheduled_command_parses() {
        use odysync_backends::scheduler::{ScheduleFrequency, ScheduleSpec, ScheduledCommand};
        for command in [ScheduledCommand::Apply, ScheduledCommand::Care] {
            let spec = ScheduleSpec {
                frequency: ScheduleFrequency::Daily,
                time: "09:00".into(),
                task_name: "Odysync".into(),
                extra_args: Vec::new(),
                program: None,
                command,
            };
            let args = std::iter::once("odysync".to_string()).chain(spec.args());
            let cli = Cli::try_parse_from(args).expect("parses");
            assert!(matches!(
                cli.command,
                Command::Apply {
                    yes: true,
                    unattended: true,
                    ..
                } | Command::Care { yes: true, .. }
            ));
        }
    }

    #[test]
    fn the_scan_line_names_who_is_still_busy() {
        let w = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            scan_line(0, 3, &w(&["a", "b", "c"])),
            "Checked 0 of 3 sources"
        );
        assert_eq!(
            scan_line(1, 3, &w(&["b", "c"])),
            "Checked 1 of 3 sources, waiting for b, c"
        );
        assert_eq!(
            scan_line(1, 4, &w(&["b", "c", "d"])),
            "Checked 1 of 4 sources, waiting for b, c ..."
        );
        assert_eq!(scan_line(3, 3, &[]), "Checked 3 of 3 sources");
    }
    use odysync_core::model::{BackendKind, PackageId};
    use odysync_core::version::Version;

    fn candidate(backend: BackendKind, id: &str) -> UpdateCandidate {
        UpdateCandidate {
            id: PackageId::new(backend, id),
            name: id.into(),
            installed: Version::parse("1.0.0"),
            available: Version::parse("2.0.0"),
            size_bytes: None,
            expected_sha256: None,
        }
    }

    #[test]
    fn filters_match_bare_and_qualified_ids_case_insensitively() {
        let c = candidate(BackendKind::Winget, "Mozilla.Firefox");
        assert!(matches_any(&c, &["Mozilla.Firefox".into()]));
        assert!(matches_any(&c, &["mozilla.firefox".into()]));
        assert!(matches_any(&c, &["winget:Mozilla.Firefox".into()]));
        assert!(!matches_any(&c, &["7zip.7zip".into()]));
    }

    #[test]
    fn a_qualified_filter_does_not_match_a_different_backend() {
        let c = candidate(BackendKind::Winget, "firefox");
        assert!(!matches_any(&c, &["homebrew:firefox".into()]));
    }

    #[test]
    fn no_filters_means_no_restriction() {
        let cfg = Config::default();
        assert!(resolve_filters(&cfg, &[], None).unwrap().is_none());
    }

    #[test]
    fn a_missing_profile_is_an_error_rather_than_an_empty_run() {
        // Silently updating everything because a profile name was typo'd would
        // be exactly the wrong failure mode.
        let cfg = Config::default();
        assert!(resolve_filters(&cfg, &[], Some("nope")).is_err());
    }

    #[test]
    fn a_profile_contributes_its_packages_as_filters() {
        let mut cfg = Config::default();
        cfg.profiles.push(odysync_core::config::Profile {
            name: "browsers".into(),
            packages: vec!["Mozilla.Firefox".into()],
        });
        let filters = resolve_filters(&cfg, &[], Some("browsers"))
            .unwrap()
            .unwrap();
        assert_eq!(filters, vec!["Mozilla.Firefox".to_string()]);
    }

    #[test]
    fn only_and_profile_filters_combine() {
        let mut cfg = Config::default();
        cfg.profiles.push(odysync_core::config::Profile {
            name: "browsers".into(),
            packages: vec!["Mozilla.Firefox".into()],
        });
        let filters = resolve_filters(&cfg, &["7zip.7zip".into()], Some("browsers"))
            .unwrap()
            .unwrap();
        assert_eq!(filters.len(), 2);
    }
}
