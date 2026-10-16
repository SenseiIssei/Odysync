//! Core domain for Odysync: model, safety policy, planning and the
//! apply runner. This crate performs no I/O against package managers — that
//! lives in `odysync-backends` — which keeps every safety rule unit-testable.

pub mod backend;
pub mod cleanup;
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
pub mod verification;
pub mod version;
pub mod windows_update;

pub use backend::{ApplyPhase, ApplyProgress, Backend};
pub use cleanup::{CleanupCategory, CleanupFinding, CleanupOutcome, Reversibility};
pub use config::Config;
pub use error::{Error, Result};
pub use health::{all_passed, failure_reasons, run_health_checks, HealthCheckResult};
pub use history::{HistoryEntry, HistoryOutcome, UpdateHistory};
pub use maintenance::{Maintenance, MaintenanceKind, MaintenanceResult};
pub use model::{
    ApplyOutcome, BackendKind, InstalledPackage, PackageId, PlannedUpdate, SkipReason,
    UpdateCandidate,
};
pub use policy::{Hold, Policy};
pub use report::RunReport;
pub use restore::RestorePointGuard;
pub use runner::{ProgressEmitter, ProgressEvent, Runner};
pub use scan_cache::ScanCache;
pub use verification::{verification_of, Verification};
pub use version::Version;
pub use windows_update::UpdateClass;
