//! The contract every package-manager integration implements.
//!
//! Keeping this trait narrow is what makes the tool modular: a backend only
//! knows how to enumerate updates and apply one package. Deciding *whether* an
//! update should happen lives entirely in [`crate::policy`], and verification
//! lives in `odysync-verify`, so no backend can accidentally opt out of a safety
//! rule.

use async_trait::async_trait;

use crate::error::Result;
use crate::model::{BackendKind, UpdateCandidate};

/// A source of software updates.
#[async_trait]
pub trait Backend: Send + Sync {
    /// Which backend this is.
    fn kind(&self) -> BackendKind;

    /// Human-readable name for the UI.
    fn display_name(&self) -> &str;

    /// Whether the underlying tool is present and usable on this machine.
    ///
    /// Called before every scan so a missing package manager degrades to "no
    /// updates from this source" instead of an error.
    async fn is_available(&self) -> bool;

    /// Enumerate packages with a newer version available.
    ///
    /// Implementations must report versions verbatim and must not filter on
    /// their own idea of what is safe — that is the policy engine's job.
    async fn scan(&self) -> Result<Vec<UpdateCandidate>>;

    /// Install exactly `candidate.available` for `candidate`.
    ///
    /// Contract that every implementation must uphold:
    ///
    ///   * pin the exact target version; never let the backend pick "latest"
    ///   * never fall back to an install/reinstall of a package that is
    ///     already present — a failed upgrade must stay failed
    ///   * never spawn a visible console window
    ///   * verify the installed version afterwards via [`Backend::installed_version`]
    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()>;

    /// Read back the version currently installed, for post-apply confirmation.
    ///
    /// Returning `Ok(None)` means "cannot tell", which the runner reports as a
    /// non-converged update rather than a success.
    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>>;
}
