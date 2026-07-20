//! Core domain for Sensei's Updater: model, safety policy, planning and the
//! apply runner. This crate performs no I/O against package managers — that
//! lives in `sensei-backends` — which keeps every safety rule unit-testable.

pub mod backend;
pub mod config;
pub mod error;
pub mod model;
pub mod platform;
pub mod policy;
pub mod proc;
pub mod report;
pub mod runner;
pub mod version;

pub use backend::Backend;
pub use config::Config;
pub use error::{Error, Result};
pub use model::{ApplyOutcome, BackendKind, PackageId, PlannedUpdate, SkipReason, UpdateCandidate};
pub use policy::{Hold, Policy};
pub use report::RunReport;
pub use runner::Runner;
pub use version::Version;
