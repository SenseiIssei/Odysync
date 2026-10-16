//! The curated list of apps Odysync can update straight from their publisher.
//!
//! ## Why a curated catalog, and not "any app with a website"
//!
//! The alternative — guess a download page from an app's name, scrape a link,
//! run whatever it points at — is how update tools end up installing adware.
//! Every entry here names three things a human checked against the real
//! publisher before it was added:
//!
//! 1. **where the version and the download come from** — a GitHub repository
//!    or a vendor's own update API, never a third-party mirror;
//! 2. **who signs the installer** — the Authenticode signer the download must
//!    carry, matched exactly. A valid signature from anybody else is refused;
//! 3. **what kind of installer it is** — so it can be run silently, and so it
//!    is only ever run over an install of the *same* kind (see
//!    [`InstallerKind::compatible_with`]).
//!
//! Apps whose publisher does not sign installers (7-Zip, AutoHotkey, PeaZip,
//! qBittorrent, ShareX and others at the time of writing) are deliberately
//! absent: with nothing to pin, the only remaining check would be "the bytes
//! match the hash next to them", which proves the download was not corrupted,
//! not who made it.
//!
//! Everything in this module is pure and runs in tests on every platform.

use std::collections::BTreeMap;

use regex::Regex;
use serde::Deserialize;

/// The catalog shipped inside the binary.
const EMBEDDED: &str = include_str!("catalog.json");

#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    pub schema: u32,
    pub apps: Vec<CatalogApp>,
}

impl Catalog {
    /// The catalog compiled into this build.
    ///
    /// Parsed on every call: it is a few kilobytes, read once per scan, and
    /// keeping it out of a global means a test can never observe another
    /// test's mutation.
    pub fn embedded() -> Catalog {
        serde_json::from_str(EMBEDDED).expect("the embedded catalog is validated by tests")
    }

    pub fn find(&self, id: &str) -> Option<&CatalogApp> {
        self.apps.iter().find(|a| a.id == id)
    }

    /// The catalog entry that describes `app`, if any.
    ///
    /// The first match wins, and the tests assert no installed program can
    /// match two entries, so order never decides anything.
    pub fn match_installed(&self, app: &InstalledApp) -> Option<&CatalogApp> {
        self.apps.iter().find(|entry| entry.matches(app))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogApp {
    /// Stable id, used in package ids, holds and the CLI.
    pub id: String,
    pub name: String,
    pub homepage: String,
    #[serde(rename = "match")]
    pub matcher: Matcher,
    pub source: Source,
    pub installer: InstallerKind,
    /// Arguments appended after the installer kind's own silent switches.
    #[serde(default)]
    pub extra_args: Vec<String>,
    /// Subject CN or O the installer's code signature must carry.
    pub signer: String,
}

impl CatalogApp {
    pub fn matches(&self, app: &InstalledApp) -> bool {
        self.matcher.matches(app) && self.installer.compatible_with(app.technology)
    }
}

/// How an entry recognises an installed program in the OS's own list.
#[derive(Debug, Clone, Deserialize)]
pub struct Matcher {
    /// Regex over the program's display name.
    pub display_name: String,
    /// Optional regex over the publisher, for names generic enough to collide.
    #[serde(default)]
    pub publisher: Option<String>,
}

impl Matcher {
    pub fn matches(&self, app: &InstalledApp) -> bool {
        let name_ok = Regex::new(&self.display_name)
            .map(|re| re.is_match(&app.display_name))
            .unwrap_or(false);
        if !name_ok {
            return false;
        }
        match &self.publisher {
            None => true,
            Some(pattern) => match (&app.publisher, Regex::new(pattern)) {
                (Some(publisher), Ok(re)) => re.is_match(publisher),
                // An entry that asks for a publisher and an app that does not
                // declare one do not match: the publisher check exists because
                // the name alone was judged too generic.
                _ => false,
            },
        }
    }
}

/// Where the latest version and its installer come from.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Source {
    /// The latest non-prerelease GitHub release of `repo`.
    Github {
        repo: String,
        /// Regex over the tag with a named group `v`, and optionally `rev`.
        tag: String,
        /// Asset-name regex per CPU architecture.
        assets: BTreeMap<String, String>,
        /// Assets for a per-user install, where the publisher ships a separate
        /// per-user installer. Falls back to `assets`.
        #[serde(default)]
        user_assets: BTreeMap<String, String>,
    },
    /// A vendor JSON endpoint per `<scope>-<arch>`, returning one object.
    Json {
        endpoints: BTreeMap<String, String>,
        version_field: String,
        url_field: String,
        #[serde(default)]
        sha256_field: Option<String>,
        /// Every download URL must start with this.
        download_prefix: String,
    },
    /// Node.js release index, staying on the installed major line.
    Nodejs,
}

/// The installer technologies Odysync knows how to run silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallerKind {
    /// Nullsoft: `/S`.
    Nsis,
    /// Inno Setup: `/VERYSILENT` and friends.
    Inno,
    /// Windows Installer, through `msiexec /i`.
    Msi,
}

impl InstallerKind {
    /// Whether an installer of this kind may be run over `installed`.
    ///
    /// Running an MSI over an app that was installed by an `.exe`, or the
    /// other way round, does not upgrade it: the two technologies keep
    /// separate records, so the result is a second copy beside the first and
    /// two entries in Settings → Apps. Audacity moving from Inno Setup to MSI
    /// between major versions is the real case this prevents. Refusing is the
    /// conservative answer — the user can still move across by hand.
    pub fn compatible_with(&self, installed: Technology) -> bool {
        matches!(
            (self, installed),
            (InstallerKind::Msi, Technology::Msi)
                | (InstallerKind::Inno, Technology::Inno)
                | (InstallerKind::Nsis, Technology::Other)
        )
    }

    /// The silent-install argument list, without the installer path itself.
    pub fn silent_args(&self) -> &'static [&'static str] {
        match self {
            InstallerKind::Nsis => &["/S"],
            // `/SP-` skips the "this will install..." prompt that older Inno
            // versions show even in silent mode; `/NORESTART` leaves the
            // reboot decision to the user, whom Odysync tells.
            InstallerKind::Inno => &["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-"],
            InstallerKind::Msi => &["/qn", "/norestart"],
        }
    }
}

/// How an installed program was installed, as far as the OS records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Technology {
    /// `WindowsInstaller = 1` on the uninstall entry.
    Msi,
    /// An uninstall key ending in `_is1`, which is Inno Setup's convention.
    Inno,
    /// Anything else — NSIS, custom installers.
    Other,
}

/// Who an installed program was installed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// `HKEY_LOCAL_MACHINE`: every user; updating needs administrator rights.
    Machine,
    /// `HKEY_CURRENT_USER`: just this user.
    User,
}

impl Scope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Scope::Machine => "machine",
            Scope::User => "user",
        }
    }
}

/// One program from the OS's list of installed software.
#[derive(Debug, Clone)]
pub struct InstalledApp {
    pub display_name: String,
    pub display_version: Option<String>,
    pub publisher: Option<String>,
    pub scope: Scope,
    /// `x64`, `x86` or `arm64`.
    pub arch: &'static str,
    pub technology: Technology,
    /// Where the program lives, when the OS knows. Used to refuse updating an
    /// app that is running.
    pub install_dir: Option<std::path::PathBuf>,
}

/// The latest release a source offers, ready to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub sha256: Option<String>,
    pub size_bytes: Option<u64>,
}

// ── Pure helpers used by the resolvers ──────────────────────────────────────

/// The version a release tag stands for, per the entry's tag pattern.
///
/// The named group `v` is the version. An optional `rev` group is a
/// distribution revision, appended only when it is not `1` — Git for Windows
/// tags `v2.56.0.windows.1` as `2.56.0` and `v2.56.0.windows.2` as
/// `2.56.0.2`, and that is exactly how its installer records the version, so
/// comparing like with like needs the same rule.
pub fn version_from_tag(pattern: &str, tag: &str) -> Option<String> {
    let re = Regex::new(pattern).ok()?;
    let caps = re.captures(tag.trim())?;
    let version = caps.name("v")?.as_str().to_string();
    match caps.name("rev").map(|m| m.as_str()) {
        None | Some("1") | Some("") => Some(version),
        Some(rev) => Some(format!("{version}.{rev}")),
    }
}

/// The asset-name pattern for this scope and architecture.
pub fn asset_pattern<'a>(
    assets: &'a BTreeMap<String, String>,
    user_assets: &'a BTreeMap<String, String>,
    scope: Scope,
    arch: &str,
) -> Option<&'a String> {
    if scope == Scope::User
        && let Some(p) = user_assets.get(arch)
    {
        return Some(p);
    }
    assets.get(arch)
}

/// A GitHub asset digest as a bare lowercase SHA-256, if it is one.
///
/// GitHub reports `"sha256:<hex>"`. Anything else — another algorithm, a
/// truncated value — is dropped rather than half-trusted.
pub fn sha256_from_digest(digest: Option<&str>) -> Option<String> {
    let hex = digest?.strip_prefix("sha256:")?;
    is_sha256_hex(hex).then(|| hex.to_ascii_lowercase())
}

pub fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether a GitHub download URL belongs to `repo`'s own releases.
///
/// Asset URLs are taken from the API response, so this is the check that a
/// response cannot point the download at a different project.
pub fn is_release_asset_of(repo: &str, url: &str) -> bool {
    let prefix = format!("https://github.com/{repo}/releases/download/");
    url.len() > prefix.len() && url[..prefix.len()].eq_ignore_ascii_case(&prefix)
}

/// The newest release on the same major line as `installed`, from Node.js's
/// `index.json`.
///
/// Node's major lines are where its breaking changes live; moving someone from
/// 22 to 24 is a decision about their projects, not a patch. So the update
/// offered is always the latest patch of the line already installed.
pub fn pick_nodejs_release(index: &serde_json::Value, installed: &str) -> Option<String> {
    let major = installed.trim().trim_start_matches('v').split('.').next()?;
    if major.is_empty() || !major.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let prefix = format!("v{major}.");
    // index.json is ordered newest first.
    index.as_array()?.iter().find_map(|release| {
        let version = release.get("version")?.as_str()?;
        version
            .starts_with(&prefix)
            .then(|| version.trim_start_matches('v').to_string())
    })
}

/// The SHA-256 for `file` from a `SHASUMS256.txt` body.
pub fn sha256_from_shasums(body: &str, file: &str) -> Option<String> {
    body.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?;
        (name == file && is_sha256_hex(hash)).then(|| hash.to_ascii_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, publisher: Option<&str>, technology: Technology) -> InstalledApp {
        InstalledApp {
            display_name: name.into(),
            display_version: Some("1.0".into()),
            publisher: publisher.map(str::to_string),
            scope: Scope::Machine,
            arch: "x64",
            technology,
            install_dir: None,
        }
    }

    #[test]
    fn the_embedded_catalog_parses_and_every_entry_is_complete() {
        let catalog = Catalog::embedded();
        assert_eq!(catalog.schema, 1);
        assert!(catalog.apps.len() >= 10);

        let mut ids: Vec<&str> = catalog.apps.iter().map(|a| a.id.as_str()).collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "two catalog entries share an id");

        for entry in &catalog.apps {
            assert!(
                entry
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                "{}: ids are lowercase-kebab",
                entry.id
            );
            assert!(!entry.signer.trim().is_empty(), "{}: no signer", entry.id);
            assert!(
                entry.homepage.starts_with("https://"),
                "{}: homepage",
                entry.id
            );
            Regex::new(&entry.matcher.display_name)
                .unwrap_or_else(|e| panic!("{}: bad name regex: {e}", entry.id));
            if let Some(p) = &entry.matcher.publisher {
                Regex::new(p).unwrap_or_else(|e| panic!("{}: bad publisher regex: {e}", entry.id));
            }
            match &entry.source {
                Source::Github {
                    repo,
                    tag,
                    assets,
                    user_assets,
                } => {
                    assert_eq!(repo.split('/').count(), 2, "{}: repo", entry.id);
                    let re = Regex::new(tag).expect("tag regex");
                    assert!(
                        re.capture_names().any(|n| n == Some("v")),
                        "{}: the tag pattern has no `v` group",
                        entry.id
                    );
                    assert!(!assets.is_empty(), "{}: no assets", entry.id);
                    for pattern in assets.values().chain(user_assets.values()) {
                        Regex::new(pattern).expect("asset regex");
                    }
                    for arch in assets.keys().chain(user_assets.keys()) {
                        assert!(
                            ["x64", "x86", "arm64"].contains(&arch.as_str()),
                            "{}: unknown arch {arch}",
                            entry.id
                        );
                    }
                }
                Source::Json {
                    endpoints,
                    download_prefix,
                    ..
                } => {
                    assert!(download_prefix.starts_with("https://"));
                    for (key, url) in endpoints {
                        assert!(url.starts_with("https://"), "{}: {key}", entry.id);
                        let (scope, arch) = key.split_once('-').expect("scope-arch");
                        assert!(["user", "machine"].contains(&scope));
                        assert!(["x64", "x86", "arm64"].contains(&arch));
                    }
                }
                Source::Nodejs => {}
            }
        }
    }

    /// A program matching two entries would make the result depend on file
    /// order. Checked against a sample of real display names.
    #[test]
    fn real_display_names_match_exactly_one_entry_or_none() {
        let catalog = Catalog::embedded();
        let cases: &[(&str, Option<&str>, Technology, Option<&str>)] = &[
            (
                "Notepad++ (64-bit x64)",
                Some("Notepad++ Team"),
                Technology::Other,
                Some("notepad-plus-plus"),
            ),
            (
                "Git",
                Some("The Git Development Community"),
                Technology::Inno,
                Some("git"),
            ),
            (
                "GitHub CLI",
                Some("GitHub, Inc."),
                Technology::Msi,
                Some("github-cli"),
            ),
            (
                "GitHub Desktop",
                Some("GitHub, Inc."),
                Technology::Other,
                None,
            ),
            ("Git LFS", None, Technology::Inno, None),
            (
                "Telegram Desktop",
                Some("Telegram FZ-LLC"),
                Technology::Inno,
                Some("telegram-desktop"),
            ),
            (
                "Microsoft Visual Studio Code (User)",
                Some("Microsoft Corporation"),
                Technology::Inno,
                Some("vscode"),
            ),
            (
                "Microsoft Visual Studio Code",
                Some("Microsoft Corporation"),
                Technology::Inno,
                Some("vscode"),
            ),
            (
                "Node.js",
                Some("Node.js Foundation"),
                Technology::Msi,
                Some("nodejs"),
            ),
            (
                "Microsoft Visual Studio Installer",
                None,
                Technology::Other,
                None,
            ),
        ];

        for (name, publisher, tech, expected) in cases {
            let installed = app(name, *publisher, *tech);
            let hits: Vec<&str> = catalog
                .apps
                .iter()
                .filter(|e| e.matches(&installed))
                .map(|e| e.id.as_str())
                .collect();
            assert!(hits.len() <= 1, "{name} matched {hits:?}");
            assert_eq!(hits.first().copied(), *expected, "{name}");
        }
    }

    #[test]
    fn an_installer_only_runs_over_an_install_of_its_own_kind() {
        assert!(InstallerKind::Msi.compatible_with(Technology::Msi));
        assert!(InstallerKind::Inno.compatible_with(Technology::Inno));
        assert!(InstallerKind::Nsis.compatible_with(Technology::Other));

        assert!(!InstallerKind::Msi.compatible_with(Technology::Inno));
        assert!(!InstallerKind::Msi.compatible_with(Technology::Other));
        assert!(!InstallerKind::Inno.compatible_with(Technology::Msi));
        assert!(!InstallerKind::Nsis.compatible_with(Technology::Msi));
        assert!(!InstallerKind::Nsis.compatible_with(Technology::Inno));
    }

    /// Audacity 3 shipped an Inno installer, Audacity 4 an MSI. Running the
    /// MSI over the old install would leave two Audacities behind.
    #[test]
    fn an_app_that_changed_installer_technology_is_not_matched() {
        let catalog = Catalog::embedded();
        let old = app("Audacity 3.7.3", Some("Audacity Team"), Technology::Inno);
        assert!(catalog.match_installed(&old).is_none());
        let new = app("Audacity 4.0.1", Some("Muse Group"), Technology::Msi);
        assert_eq!(
            catalog.match_installed(&new).map(|e| e.id.as_str()),
            Some("audacity")
        );
    }

    #[test]
    fn a_required_publisher_that_is_missing_does_not_match() {
        let catalog = Catalog::embedded();
        let anonymous = app("Git", None, Technology::Inno);
        assert!(catalog.match_installed(&anonymous).is_none());
    }

    #[test]
    fn tags_become_versions_the_way_the_installer_records_them() {
        let git = r"^v(?P<v>\d+\.\d+\.\d+)\.windows\.(?P<rev>\d+)$";
        assert_eq!(
            version_from_tag(git, "v2.56.0.windows.1").as_deref(),
            Some("2.56.0")
        );
        assert_eq!(
            version_from_tag(git, "v2.56.0.windows.2").as_deref(),
            Some("2.56.0.2")
        );
        assert_eq!(version_from_tag(git, "v2.56.0-rc1.windows.1"), None);

        let plain = r"^v(?P<v>\d+(?:\.\d+)+)$";
        assert_eq!(
            version_from_tag(plain, "v8.9.8.1").as_deref(),
            Some("8.9.8.1")
        );
        assert_eq!(version_from_tag(plain, "nightly"), None);
        assert_eq!(
            version_from_tag(r"^Audacity-(?P<v>\d+\.\d+\.\d+)$", "Audacity-4.0.1").as_deref(),
            Some("4.0.1")
        );
    }

    #[test]
    fn per_user_installs_prefer_the_per_user_asset() {
        let mut assets = BTreeMap::new();
        assets.insert("x64".to_string(), "machine".to_string());
        let mut user = BTreeMap::new();
        user.insert("x64".to_string(), "user".to_string());

        assert_eq!(
            asset_pattern(&assets, &user, Scope::User, "x64").map(String::as_str),
            Some("user")
        );
        assert_eq!(
            asset_pattern(&assets, &user, Scope::Machine, "x64").map(String::as_str),
            Some("machine")
        );
        // No per-user installer published: the ordinary one is used.
        assert_eq!(
            asset_pattern(&assets, &BTreeMap::new(), Scope::User, "x64").map(String::as_str),
            Some("machine")
        );
        // No asset for this architecture: nothing is offered, rather than a
        // different architecture's installer.
        assert_eq!(asset_pattern(&assets, &user, Scope::Machine, "arm64"), None);
    }

    #[test]
    fn only_well_formed_sha256_digests_are_kept() {
        let hex = "a".repeat(64);
        assert_eq!(
            sha256_from_digest(Some(&format!("sha256:{hex}"))),
            Some(hex.clone())
        );
        assert_eq!(
            sha256_from_digest(Some(&format!("sha256:{}", hex.to_uppercase()))),
            Some(hex.clone())
        );
        assert_eq!(sha256_from_digest(Some(&format!("sha512:{hex}"))), None);
        assert_eq!(sha256_from_digest(Some("sha256:abc")), None);
        assert_eq!(sha256_from_digest(None), None);
    }

    #[test]
    fn a_download_url_must_belong_to_the_repository_named_in_the_catalog() {
        let repo = "notepad-plus-plus/notepad-plus-plus";
        assert!(is_release_asset_of(
            repo,
            "https://github.com/notepad-plus-plus/notepad-plus-plus/releases/download/v8.9.8.1/npp.exe"
        ));
        assert!(!is_release_asset_of(
            repo,
            "https://github.com/someone-else/notepad-plus-plus/releases/download/v1/npp.exe"
        ));
        assert!(!is_release_asset_of(
            repo,
            "http://github.com/notepad-plus-plus/notepad-plus-plus/releases/download/v1/npp.exe"
        ));
        assert!(!is_release_asset_of(
            repo,
            "https://github.com.evil.example/notepad-plus-plus/notepad-plus-plus/releases/download/v1/x"
        ));
    }

    #[test]
    fn nodejs_stays_on_the_installed_major_line() {
        let index = serde_json::json!([
            { "version": "v25.1.0" },
            { "version": "v24.22.0" },
            { "version": "v24.21.0" },
            { "version": "v22.20.0" }
        ]);
        assert_eq!(
            pick_nodejs_release(&index, "24.18.0").as_deref(),
            Some("24.22.0")
        );
        assert_eq!(
            pick_nodejs_release(&index, "22.1.0").as_deref(),
            Some("22.20.0")
        );
        assert_eq!(pick_nodejs_release(&index, "20.0.0"), None);
        assert_eq!(pick_nodejs_release(&index, "garbage"), None);
    }

    #[test]
    fn shasums_lookup_matches_the_whole_file_name() {
        let body = format!(
            "{a}  node-v24.22.0-x64.msi\n{b}  node-v24.22.0-x64.msi.sig\n",
            a = "1".repeat(64),
            b = "2".repeat(64)
        );
        assert_eq!(
            sha256_from_shasums(&body, "node-v24.22.0-x64.msi"),
            Some("1".repeat(64))
        );
        assert_eq!(sha256_from_shasums(&body, "node-v24.22.0-arm64.msi"), None);
    }
}
