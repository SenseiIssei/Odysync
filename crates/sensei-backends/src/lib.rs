//! Package-manager integrations and host detection.
//!
//! Backends are discovered at runtime rather than chosen at compile time: a
//! single binary ships every integration for its platform and simply reports
//! "not available" for the ones the machine does not have. Adding a package
//! manager means implementing [`Backend`] and adding one line to
//! [`detect_backends`] — nothing else in the codebase changes.

pub mod apt;
pub mod diagnostics;
pub mod flatpak;
pub mod homebrew;
pub mod maintenance;
pub mod scheduler;
#[cfg(windows)]
pub mod winget;
#[cfg(windows)]
pub mod windows_drivers;

use sensei_core::backend::Backend;
use sensei_core::config::Config;

/// Every backend compiled into this build, whether usable here or not.
fn all_backends() -> Vec<Box<dyn Backend>> {
    let mut v: Vec<Box<dyn Backend>> = Vec::new();

    #[cfg(windows)]
    {
        v.push(Box::new(winget::WingetBackend::new()));
        v.push(Box::new(winget::WingetBackend::store()));
        v.push(Box::new(windows_drivers::WindowsDriverBackend::new()));
    }

    // Homebrew also runs on Linux, so it is not gated to macOS.
    v.push(Box::new(homebrew::HomebrewBackend::new()));
    v.push(Box::new(apt::AptBackend::new()));
    v.push(Box::new(flatpak::FlatpakBackend::new()));

    v
}

/// Backends that are present on this machine and enabled in `config`.
///
/// Availability probes run concurrently — each shells out to a package manager
/// and they are independent, so doing them in sequence would make startup as
/// slow as the sum of every probe.
pub async fn detect_backends(config: &Config) -> Vec<Box<dyn Backend>> {
    let candidates: Vec<Box<dyn Backend>> = all_backends()
        .into_iter()
        .filter(|b| config.backend_enabled(b.kind()))
        .collect();

    let results = futures::future::join_all(candidates.iter().map(|b| b.is_available())).await;

    candidates
        .into_iter()
        .zip(results)
        .filter_map(|(backend, available)| {
            if available {
                Some(backend)
            } else {
                tracing::debug!(backend = %backend.kind(), "not available on this host");
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sensei_core::model::BackendKind;

    #[test]
    fn the_build_includes_the_expected_backends_for_this_platform() {
        let kinds: Vec<BackendKind> = all_backends().iter().map(|b| b.kind()).collect();

        assert!(kinds.contains(&BackendKind::Homebrew));
        assert!(kinds.contains(&BackendKind::Apt));
        assert!(kinds.contains(&BackendKind::Flatpak));

        #[cfg(windows)]
        {
            assert!(kinds.contains(&BackendKind::Winget));
            assert!(kinds.contains(&BackendKind::MsStore));
            assert!(kinds.contains(&BackendKind::WindowsDrivers));
        }
    }

    #[test]
    fn every_backend_reports_a_non_empty_display_name() {
        for b in all_backends() {
            assert!(!b.display_name().is_empty(), "{} has no name", b.kind());
        }
    }

    #[tokio::test]
    async fn disabled_backends_are_excluded_from_detection() {
        let cfg = Config {
            disabled_backends: all_backends()
                .iter()
                .map(|b| b.kind().id().to_string())
                .collect(),
            ..Config::default()
        };

        assert!(detect_backends(&cfg).await.is_empty());
    }
}
