//! A safe, narrow wrapper over the Windows Update Agent COM API.
//!
//! Both the driver backend and the Windows Update backends drive the same
//! `IUpdateSession`; before this module existed the driver backend carried its
//! own copy of the COM plumbing, and three defects lived in that copy:
//!
//!   * **No download step.** It went straight from `SetUpdates` to `Install`.
//!     `IUpdateInstaller::Install` requires `IsDownloaded == true` and fails
//!     with `WU_E_NOT_DOWNLOADED` otherwise, so an install only ever succeeded
//!     when Windows happened to have cached the update already.
//!   * **No EULA acceptance.** An update whose licence had not been accepted
//!     could never install.
//!   * **`RebootRequired` was read and discarded**, which is why
//!     `RunReport::reboot_required` was never once set to `true`.
//!
//! A fourth was found by running the real API against a live machine: a search
//! can return `orcFailed` *and* a populated collection at the same time. The
//! old code returned early on `orcFailed` and threw those results away. On the
//! machine this was written against that meant discarding 14 pending updates,
//! 10 of them security patches.
//!
//! Everything here is `cfg(windows)`. The interesting decision — what kind of
//! update this is — lives in `odysync_core::windows_update` so it stays
//! testable on every platform.

use std::time::Duration;

use odysync_core::error::{Error, Result};
use odysync_core::windows_update::{classify, UpdateClass};

/// One update as the Windows Update Agent describes it.
#[derive(Debug, Clone)]
pub struct WuaUpdate {
    /// `IUpdateIdentity::UpdateID`, a GUID.
    pub id: String,
    /// `IUpdateIdentity::RevisionNumber`. A given UpdateID is revised in place.
    pub revision: i32,
    pub title: String,
    /// `ICategory::CategoryID` for every category, used for classification.
    pub category_ids: Vec<String>,
    /// KB article numbers, e.g. `5101650`. Usually zero or one.
    pub kb_ids: Vec<String>,
    pub is_downloaded: bool,
    pub eula_accepted: bool,
    pub size_bytes: Option<u64>,
    pub class: UpdateClass,
}

impl WuaUpdate {
    /// The stable handle used as a `PackageId::native`.
    ///
    /// The UpdateID alone is not enough: Microsoft revises an update in place
    /// and bumps `RevisionNumber`, so two offerings can share a UpdateID.
    pub fn key(&self) -> String {
        format!("{}.{}", self.id, self.revision)
    }

    /// `KB5101650` when there is a KB number, else the title.
    pub fn short_name(&self) -> String {
        match self.kb_ids.first() {
            Some(kb) => format!("KB{kb}"),
            None => self.title.clone(),
        }
    }
}

/// What an install attempt did to the machine.
#[derive(Debug, Clone, Copy, Default)]
pub struct InstallOutcome {
    pub reboot_required: bool,
}

/// Search criteria for everything Windows Update offers as software.
///
/// `IsInstalled=0` alone would also return drivers; `Type='Software'` is what
/// separates the two halves. `IsHidden=0` respects updates the user hid in the
/// Windows Update UI — hiding an update there is an explicit "not this one",
/// and a third-party tool has no business overriding it.
pub const CRITERIA_SOFTWARE: &str = "IsInstalled=0 and Type='Software' and IsHidden=0";

/// Search criteria for driver updates.
pub const CRITERIA_DRIVER: &str = "IsInstalled=0 and Type='Driver' and IsHidden=0";

/// How long an install may run before we stop waiting on it.
///
/// A cumulative update genuinely can take twenty minutes on a slow disk, so
/// this is generous. It exists to stop a wedged Windows Update service from
/// pinning a background daemon forever, not to police slow hardware.
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// Turn a Windows Update `HRESULT` into something a user can act on.
///
/// A bare `0x8024402C` tells nobody anything. These are the codes that come up
/// in practice; anything else falls through to the raw value, which is still
/// better than nothing when it reaches a bug report.
pub fn explain_hresult(hr: i32) -> String {
    let code = hr as u32;
    let detail = match code {
        0x8024_0024 => "no updates were applicable — Windows may have applied them already",
        0x8024_0034 => "the update was not downloaded, so it could not be installed",
        0x8024_0016 => "another installation is already in progress; try again once it finishes",
        0x8024_002E => "Windows Update is managed by policy and refuses non-managed changes",
        0x8024_0438 | 0x8024_402C | 0x8024_401C => {
            "could not reach the Windows Update service — check the network or a proxy"
        }
        0x8024_000B => "the operation was cancelled",
        // Seen for real: UAC on a domain-joined machine can elevate into a
        // *different* account from the interactive one, and the Update Agent
        // refuses to start its server outside the interactive session. The
        // fix is to run elevated as the signed-in user, not to retry.
        0x8024_001E => {
            "the Windows Update service stopped or would not start for this session — \
             this happens when running elevated as a different account than the one \
             signed in, and when the service has been stopped"
        }
        0x8024_001F => "no network connection is available for Windows Update",
        0x8024_1001 => "the Windows Update installer is busy or in an unexpected state",
        0x8007_0005 => "access denied — this needs to run elevated",
        0x8024_0032 => "the search criteria were rejected by the Windows Update Agent",
        0x8024_002D => "the update needs installation media that is not available",
        0x8024_A000 | 0x8024_A005 => "Windows Update is paused or disabled by policy",
        _ => return format!("Windows Update error 0x{code:08X}"),
    };
    format!("{detail} (0x{code:08X})")
}

#[cfg(windows)]
mod imp {
    use super::*;

    use windows::core::BSTR;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::UpdateAgent::{
        IUpdate, IUpdateCollection, IUpdateSession, UpdateCollection, UpdateSession,
    };

    /// `OperationResultCode::orcFailed`.
    const ORC_FAILED: i32 = 2;
    /// `OperationResultCode::orcSucceededWithErrors`.
    const ORC_SUCCEEDED_WITH_ERRORS: i32 = 3;

    /// Wrap a WUA COM failure, routing the code through [`explain_hresult`].
    ///
    /// This used to take `impl Display` and format the raw `windows::core::Error`,
    /// which meant the translation table above was only ever reached by the
    /// download and install *result codes* — never by a failure of the search
    /// itself, which is the most common way this goes wrong. A real elevated
    /// run reported `the update search failed: 0x8024001E` and left the user
    /// with a hex number and nothing to do about it.
    fn wua_err(what: &str, e: windows::core::Error) -> Error {
        Error::parse(
            "Windows Update Agent",
            format!("{what}: {}", explain_hresult(e.code().0)),
        )
    }

    /// Run `f` on a blocking thread inside a COM apartment.
    ///
    /// Every WUA interface is apartment-threaded and none of them are `Send`,
    /// so the whole interaction has to happen on one thread. `spawn_blocking`
    /// keeps that off the async runtime's worker pool, where a twenty-minute
    /// install would otherwise starve every other task.
    async fn with_com<T, F>(f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T> + Send + 'static,
    {
        tokio::task::spawn_blocking(move || {
            let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
            let result = f();
            if initialized {
                unsafe { CoUninitialize() };
            }
            result
        })
        .await
        // A `JoinError` carries no HRESULT, so it does not go through
        // `wua_err` — there is nothing for the translation table to explain.
        .map_err(|e| {
            Error::parse(
                "Windows Update Agent",
                format!("the COM worker thread panicked: {e}"),
            )
        })?
    }

    /// Create an `IUpdateSession`.
    ///
    /// `CLSCTX_ALL`, not `CLSCTX_LOCAL_SERVER`, and the difference is the whole
    /// reason Windows Update appeared to need elevation. `Microsoft.Update.Session`
    /// is registered both as an in-process server (`wuapi.dll`) and as a local
    /// server; asking specifically for the local server forces out-of-process
    /// activation against the Windows Update service, which an unelevated
    /// caller may not do — `E_ACCESSDENIED`, 0x80070005.
    ///
    /// The old driver backend hard-coded `CLSCTX_LOCAL_SERVER` and then
    /// concluded from the resulting access-denied that "the search needs
    /// elevation", which is why it hid itself from every non-admin user. The
    /// search does not: PowerShell's `New-Object -ComObject
    /// Microsoft.Update.Session` uses `CLSCTX_ALL`, activates in-process, and
    /// enumerates pending updates unelevated without complaint.
    fn create_session() -> Result<IUpdateSession> {
        unsafe { CoCreateInstance(&UpdateSession, None, CLSCTX_ALL) }
            .map_err(|e| wua_err("could not create IUpdateSession", e))
    }

    /// Read every field we care about off one `IUpdate`.
    fn read_update(update: &IUpdate) -> Result<WuaUpdate> {
        let title = unsafe { update.Title() }
            .map(|t| t.to_string())
            .unwrap_or_default();

        let identity =
            unsafe { update.Identity() }.map_err(|e| wua_err("could not read Identity", e))?;
        let id = unsafe { identity.UpdateID() }
            .map(|t| t.to_string())
            .unwrap_or_default();
        let revision = unsafe { identity.RevisionNumber() }.unwrap_or(0);

        let mut category_ids = Vec::new();
        if let Ok(categories) = unsafe { update.Categories() } {
            let count = unsafe { categories.Count() }.unwrap_or(0);
            for i in 0..count {
                if let Ok(category) = unsafe { categories.get_Item(i) } {
                    if let Ok(cid) = unsafe { category.CategoryID() } {
                        category_ids.push(cid.to_string());
                    }
                }
            }
        }

        let mut kb_ids = Vec::new();
        if let Ok(kbs) = unsafe { update.KBArticleIDs() } {
            let count = unsafe { kbs.Count() }.unwrap_or(0);
            for i in 0..count {
                if let Ok(kb) = unsafe { kbs.get_Item(i) } {
                    kb_ids.push(kb.to_string());
                }
            }
        }

        let is_downloaded = unsafe { update.IsDownloaded() }
            .map(|b| b.as_bool())
            .unwrap_or(false);
        let eula_accepted = unsafe { update.EulaAccepted() }
            .map(|b| b.as_bool())
            .unwrap_or(false);

        Ok(WuaUpdate {
            class: classify(&title, &category_ids),
            id,
            revision,
            title,
            category_ids,
            kb_ids,
            is_downloaded,
            eula_accepted,
            // `MaxDownloadSize` is a DECIMAL. Reinterpreting its bytes as a
            // u64 would be undefined behaviour, and a correct conversion needs
            // an OLE automation dependency this crate does not carry. Left
            // unreported rather than reported wrongly.
            size_bytes: None,
        })
    }

    /// Search Windows Update, returning everything it offers for `criteria`.
    pub async fn search(criteria: &'static str) -> Result<Vec<WuaUpdate>> {
        with_com(move || {
            let session = create_session()?;
            let searcher = unsafe { session.CreateUpdateSearcher() }
                .map_err(|e| wua_err("could not create IUpdateSearcher", e))?;

            let result = unsafe { searcher.Search(&BSTR::from(criteria)) }
                .map_err(|e| wua_err("the update search failed", e))?;

            let updates = unsafe { result.Updates() }
                .map_err(|e| wua_err("could not read the Updates collection", e))?;
            let count = unsafe { updates.Count() }
                .map_err(|e| wua_err("could not count the search results", e))?;

            // A failed ResultCode does *not* imply an empty collection. On a
            // real machine this search returned orcFailed alongside 14 valid
            // updates — one source in the aggregate had failed while the rest
            // succeeded. Reporting the partial result and logging the failure
            // beats discarding work Windows already did.
            let code = unsafe { result.ResultCode() }.map(|c| c.0).unwrap_or(0);
            if code == ORC_FAILED || code == ORC_SUCCEEDED_WITH_ERRORS {
                if count == 0 {
                    tracing::info!(
                        criteria,
                        "the Windows Update search failed and returned nothing; \
                         treating it as no updates available from this source"
                    );
                    return Ok(Vec::new());
                }
                tracing::warn!(
                    criteria,
                    count,
                    "the Windows Update search reported a failure but returned \
                     results; using them and reporting the list as partial"
                );
            }

            let mut out: Vec<WuaUpdate> = Vec::new();
            for i in 0..count {
                let update = unsafe { updates.get_Item(i) }
                    .map_err(|e| wua_err(&format!("could not read search result {i}"), e))?;
                let read = read_update(&update)?;

                // Windows offers the same Defender signature twice on some
                // machines (once per servicing channel). Deduplicating on the
                // identity keeps the same update from being listed, and
                // installed, twice.
                if out
                    .iter()
                    .any(|u| u.id == read.id && u.revision == read.revision)
                {
                    continue;
                }
                out.push(read);
            }

            Ok(out)
        })
        .await
    }

    /// Accept the licence, download if needed, then install one update.
    ///
    /// `key` is [`WuaUpdate::key`]. The update is re-searched rather than held
    /// across the call: COM interfaces are not `Send`, and an offering can be
    /// superseded between scan and apply — in which case refusing is correct.
    pub async fn install(criteria: &'static str, key: String) -> Result<InstallOutcome> {
        with_com(move || {
            let session = create_session()?;
            let searcher = unsafe { session.CreateUpdateSearcher() }
                .map_err(|e| wua_err("could not create IUpdateSearcher", e))?;

            let result = unsafe { searcher.Search(&BSTR::from(criteria)) }
                .map_err(|e| wua_err("the update search failed before installing", e))?;
            let updates = unsafe { result.Updates() }
                .map_err(|e| wua_err("could not read the Updates collection", e))?;
            let count = unsafe { updates.Count() }
                .map_err(|e| wua_err("could not count the search results", e))?;

            let mut found: Option<IUpdate> = None;
            for i in 0..count {
                let update = unsafe { updates.get_Item(i) }
                    .map_err(|e| wua_err(&format!("could not read search result {i}"), e))?;
                if read_update(&update)?.key() == key {
                    found = Some(update);
                    break;
                }
            }

            let Some(update) = found else {
                return Err(Error::Verification {
                    package: key.clone(),
                    detail: "the update was offered during the scan but Windows no longer \
                             offers it; it was most likely superseded"
                        .into(),
                });
            };

            // The licence has to be accepted before the download, not before
            // the install: an unaccepted EULA makes the download a no-op and
            // the install then fails as "not downloaded".
            let eula_accepted = unsafe { update.EulaAccepted() }
                .map(|b| b.as_bool())
                .unwrap_or(false);
            if !eula_accepted {
                unsafe { update.AcceptEula() }
                    .map_err(|e| wua_err("could not accept the update licence", e))?;
            }

            let single: IUpdateCollection =
                unsafe { CoCreateInstance(&UpdateCollection, None, CLSCTX_ALL) }
                    .map_err(|e| wua_err("could not create an UpdateCollection", e))?;
            unsafe { single.Add(&update) }
                .map_err(|e| wua_err("could not add the update to a collection", e))?;

            let is_downloaded = unsafe { update.IsDownloaded() }
                .map(|b| b.as_bool())
                .unwrap_or(false);
            if !is_downloaded {
                let downloader = unsafe { session.CreateUpdateDownloader() }
                    .map_err(|e| wua_err("could not create IUpdateDownloader", e))?;
                unsafe { downloader.SetUpdates(&single) }
                    .map_err(|e| wua_err("could not hand the update to the downloader", e))?;

                let download = unsafe { downloader.Download() }
                    .map_err(|e| wua_err("the download failed", e))?;
                let code = unsafe { download.ResultCode() }.map(|c| c.0).unwrap_or(0);
                if code == ORC_FAILED {
                    let hr = unsafe { download.HResult() }.unwrap_or(0);
                    return Err(Error::parse(
                        "Windows Update Agent",
                        format!("download failed: {}", explain_hresult(hr)),
                    ));
                }
            }

            let installer = unsafe { session.CreateUpdateInstaller() }
                .map_err(|e| wua_err("could not create IUpdateInstaller", e))?;
            unsafe { installer.SetUpdates(&single) }
                .map_err(|e| wua_err("could not hand the update to the installer", e))?;

            // Deliberately not `SetIsForced(true)`: forcing bypasses the
            // agent's own applicability and compatibility checks, which is how
            // a driver ends up installed against hardware it does not match.

            let install = unsafe { installer.Install() }
                .map_err(|e| wua_err("the install could not be started", e))?;

            let code = unsafe { install.ResultCode() }.map(|c| c.0).unwrap_or(0);
            if code == ORC_FAILED || code == ORC_SUCCEEDED_WITH_ERRORS {
                let hr = unsafe { install.HResult() }.unwrap_or(0);
                return Err(Error::parse(
                    "Windows Update Agent",
                    format!("install failed: {}", explain_hresult(hr)),
                ));
            }

            Ok(InstallOutcome {
                reboot_required: unsafe { install.RebootRequired() }
                    .map(|b| b.as_bool())
                    .unwrap_or(false),
            })
        })
        .await
    }
}

#[cfg(windows)]
pub use imp::{install, search};

// Non-Windows stubs. The backends that call these are already `cfg(windows)`;
// these exist so the module itself compiles everywhere and the tests below can
// run on Linux and macOS CI.
#[cfg(not(windows))]
pub async fn search(_criteria: &'static str) -> Result<Vec<WuaUpdate>> {
    Ok(Vec::new())
}

#[cfg(not(windows))]
pub async fn install(_criteria: &'static str, _key: String) -> Result<InstallOutcome> {
    Ok(InstallOutcome::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The code a real elevated run produced. It reached the user as the bare
    /// string `0x8024001E`, which tells them nothing they can act on — and the
    /// cause is specific and fixable: UAC on a domain-joined machine can
    /// elevate into a different account from the interactive one.
    #[test]
    fn the_service_stop_code_is_explained_rather_than_printed_raw() {
        let text = explain_hresult(0x8024_001Eu32 as i32);
        assert!(text.contains("0x8024001E"), "the raw code is still useful");
        assert!(
            text.contains("different account") || text.contains("stopped"),
            "not actionable: {text}"
        );
    }

    #[test]
    fn an_unknown_code_still_reports_its_hex_value() {
        // Falling through must not swallow the code — a bug report needs it.
        let text = explain_hresult(0x8024_DEADu32 as i32);
        assert!(text.contains("0x8024DEAD"), "{text}");
    }

    #[test]
    fn every_explained_code_keeps_its_hex_value_in_the_message() {
        // The explanation is for the user; the hex is for the bug report.
        // Both have to survive, and it is easy to add a case that drops one.
        for code in [
            0x8024_0024u32,
            0x8024_0034,
            0x8024_0016,
            0x8024_002E,
            0x8024_001E,
            0x8024_001F,
            0x8007_0005,
        ] {
            let text = explain_hresult(code as i32);
            assert!(
                text.contains(&format!("0x{code:08X}")),
                "0x{code:08X} lost its code: {text}"
            );
            assert!(text.len() > 20, "0x{code:08X} has a stub explanation");
        }
    }

    #[test]
    fn the_software_criteria_excludes_drivers_and_hidden_updates() {
        assert!(CRITERIA_SOFTWARE.contains("Type='Software'"));
        assert!(CRITERIA_SOFTWARE.contains("IsInstalled=0"));
        assert!(CRITERIA_SOFTWARE.contains("IsHidden=0"));
        assert!(!CRITERIA_SOFTWARE.contains("Driver"));
    }

    #[test]
    fn the_driver_criteria_excludes_software_and_hidden_updates() {
        assert!(CRITERIA_DRIVER.contains("Type='Driver'"));
        assert!(CRITERIA_DRIVER.contains("IsHidden=0"));
        assert!(!CRITERIA_DRIVER.contains("Software"));
    }

    fn update(id: &str, revision: i32, kb: &[&str]) -> WuaUpdate {
        WuaUpdate {
            id: id.into(),
            revision,
            title: "A title".into(),
            category_ids: Vec::new(),
            kb_ids: kb.iter().map(|s| s.to_string()).collect(),
            is_downloaded: false,
            eula_accepted: false,
            size_bytes: None,
            class: UpdateClass::Quality,
        }
    }

    #[test]
    fn the_key_includes_the_revision() {
        // Microsoft revises an update in place, so the UpdateID alone would
        // let a stale revision satisfy a request for a newer one.
        let a = update("11111111-2222-3333-4444-555555555555", 1, &[]);
        let b = update("11111111-2222-3333-4444-555555555555", 2, &[]);
        assert_ne!(a.key(), b.key());
        assert!(a.key().ends_with(".1"));
    }

    #[test]
    fn the_short_name_prefers_the_kb_number() {
        assert_eq!(update("id", 1, &["5101650"]).short_name(), "KB5101650");
        assert_eq!(update("id", 1, &[]).short_name(), "A title");
    }

    #[test]
    fn known_hresults_are_explained_and_unknown_ones_are_still_reported() {
        let not_downloaded = explain_hresult(0x8024_0034u32 as i32);
        assert!(not_downloaded.contains("not downloaded"));
        assert!(not_downloaded.contains("0x80240034"));

        let denied = explain_hresult(0x8007_0005u32 as i32);
        assert!(denied.contains("elevated"));

        // An unmapped code must still round-trip its value rather than
        // collapsing into a useless "unknown error".
        let unknown = explain_hresult(0x8024_DEADu32 as i32);
        assert!(unknown.contains("0x8024DEAD"));
    }
}
