//! Core domain for Odysync: model, safety policy, planning and the
//! apply runner. This crate performs no I/O against package managers — that
//! lives in `odysync-backends` — which keeps every safety rule unit-testable.

pub mod backend;
pub mod config;
pub mod error;
pub mod health;
pub mod history;
pub mod maintenance;
pub mod model;
pub mod platform;
pub mod policy;
pub mod proc;
pub mod report;
pub mod restore;
pub mod runner;
pub mod scan_cache;
pub mod version;

pub use backend::{ApplyPhase, ApplyProgress, Backend};
pub use config::Config;
pub use error::{Error, Result};
pub use health::{run_health_checks, all_passed, failure_reasons, HealthCheckResult};
pub use history::{HistoryEntry, HistoryOutcome, UpdateHistory};
pub use maintenance::{Maintenance, MaintenanceKind, MaintenanceResult};
pub use model::{ApplyOutcome, BackendKind, PackageId, PlannedUpdate, SkipReason, UpdateCandidate};
pub use policy::{Hold, Policy};
pub use report::RunReport;
pub use restore::RestorePointGuard;
pub use runner::{Runner, ProgressEmitter, ProgressEvent};
pub use scan_cache::ScanCache;
pub use version::Version;
