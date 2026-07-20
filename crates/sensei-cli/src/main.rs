//! `sensei` — the cross-platform command line for Sensei's Updater.
//!
//! The CLI is a thin shell over `sensei-core`: it gathers candidates from every
//! detected backend, runs them through the policy engine, and applies whatever
//! survives. Every safety decision belongs to the core, so the CLI and the
//! forthcoming GUI cannot drift apart in what they consider safe.

mod render;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use sensei_core::config::Config;
use sensei_core::model::UpdateCandidate;
use sensei_core::platform;
use sensei_core::report::RunReport;
use sensei_core::runner::Runner;
use sensei_core::Backend;

use render::Style;

#[derive(Parser)]
#[command(
    name = "sensei",
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

        /// Limit to a named profile from the config.
        #[arg(long)]
        profile: Option<String>,

        /// Write a JSON run report here.
        #[arg(long)]
        report: Option<PathBuf>,
    },

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

    /// Show the resolved configuration and where it lives.
    Config,
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
                .unwrap_or_else(|_| format!("sensei={level}").into()),
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

    let style = Style::detect();

    match cli.command {
        Command::Backends => {
            let backends = sensei_backends::detect_backends(&config).await;
            if cli.json {
                let list: Vec<_> = backends
                    .iter()
                    .map(|b| {
                        serde_json::json!({
                            "id": b.kind().id(),
                            "name": b.display_name(),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else if backends.is_empty() {
                println!("No supported package managers were found on this system.");
            } else {
                println!("{}", style.bold("Detected package managers\n"));
                for b in &backends {
                    println!("  {:<16}  {}", b.kind().id(), b.display_name());
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

        Command::Scan { show_skipped } => {
            let backends = sensei_backends::detect_backends(&config).await;
            let candidates = scan_all(&backends).await;
            let plan = config.policy.plan(candidates);

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            } else {
                print!("{}", render::plan_table(&plan, &style, show_skipped));
            }
            Ok(0)
        }

        Command::Apply {
            yes,
            dry_run,
            only,
            profile,
            report: report_path,
        } => {
            let backends = sensei_backends::detect_backends(&config).await;
            let mut candidates = scan_all(&backends).await;

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
                return Ok(0);
            }

            if !yes && !dry_run && !confirm(actionable)? {
                println!("Cancelled.");
                return Ok(0);
            }

            let refs: Vec<&dyn Backend> = backends.iter().map(|b| b.as_ref()).collect();
            let runner = Runner::new(refs, dry_run);
            let mut report = RunReport::new();
            runner.run(&plan, &mut report).await;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", render::summary(&report, &style));
            }

            if let Some(path) = report_path {
                std::fs::write(&path, serde_json::to_string_pretty(&report)?)
                    .with_context(|| format!("writing report to {}", path.display()))?;
            }

            Ok(report.exit_code() as u8)
        }

        Command::Hold { package, pin, note } => {
            config
                .policy
                .holds
                .retain(|h| !h.package.eq_ignore_ascii_case(&package));
            config.policy.holds.push(sensei_core::policy::Hold {
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
    }
}

/// Scan every backend concurrently.
///
/// A backend that errors is logged and contributes nothing rather than failing
/// the whole run — one broken package manager must not hide updates available
/// from the others.
async fn scan_all(backends: &[Box<dyn Backend>]) -> Vec<UpdateCandidate> {
    let results = futures::future::join_all(backends.iter().map(|b| async move {
        match b.scan().await {
            Ok(found) => {
                tracing::info!(backend = %b.kind(), count = found.len(), "scan complete");
                found
            }
            Err(e) => {
                tracing::warn!(backend = %b.kind(), error = %e, "scan failed");
                eprintln!("warning: {} scan failed: {e}", b.display_name());
                Vec::new()
            }
        }
    }))
    .await;

    results.into_iter().flatten().collect()
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
    })
}

/// Ask before changing the system.
fn confirm(count: usize) -> Result<bool> {
    use std::io::{BufRead, Write};

    print!("\nInstall {count} update(s)? [y/N] ");
    std::io::stdout().flush()?;

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
    use sensei_core::model::{BackendKind, PackageId};
    use sensei_core::version::Version;

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
        cfg.profiles.push(sensei_core::config::Profile {
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
        cfg.profiles.push(sensei_core::config::Profile {
            name: "browsers".into(),
            packages: vec!["Mozilla.Firefox".into()],
        });
        let filters = resolve_filters(&cfg, &["7zip.7zip".into()], Some("browsers"))
            .unwrap()
            .unwrap();
        assert_eq!(filters.len(), 2);
    }
}
