//! Language runtime package managers: pip, cargo, npm, go, PowerShell modules.
//!
//! Every one of these installs an exact version, and every one of them would
//! also install a package that is not there, or move one backwards, when asked
//! to. So `apply` first reads the installed version again and refuses unless it
//! is older than the approved one; see `guard`.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use async_trait::async_trait;
use odysync_core::backend::Backend;
use odysync_core::error::{Error, Result};
use odysync_core::model::{BackendKind, InstalledPackage, PackageId, UpdateCandidate};
use odysync_core::proc;
use odysync_core::version::Version;
use odysync_core::version_native::Scheme;

use crate::guard;

/// Listing what is installed is local and quick.
const QUERY_TIMEOUT: Duration = Duration::from_secs(60);
/// pip asks the index about every installed package, one request each.
const SCAN_TIMEOUT: Duration = Duration::from_secs(180);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// `cargo install` compiles the crate and everything it depends on.
const COMPILE_TIMEOUT: Duration = Duration::from_secs(45 * 60);

/// Check that `apply` turns away ids that would be read as options, and an
/// unknown target. None of these may reach the tool, so this is safe to run
/// where the tool is installed.
#[cfg(test)]
pub(crate) async fn assert_refuses_bad_input(
    backend: &dyn Backend,
    good_id: &str,
    scheme: Option<Scheme>,
) {
    let parse = |v: &str| match scheme {
        Some(s) => Version::parse_as(v, s),
        None => Version::parse(v),
    };
    let candidate = |id: &str, available: Version| UpdateCandidate {
        id: PackageId::new(backend.kind(), id),
        name: id.into(),
        installed: parse("1.0"),
        available,
        size_bytes: None,
        expected_sha256: None,
    };
    for id in ["-x", "--force", " -y", "", "a;b", "a/../b"] {
        let err = backend
            .apply(&candidate(id, parse("2.0")))
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidPackageId { .. }),
            "{id:?} was not refused as an id: {err}"
        );
    }
    let err = backend
        .apply(&candidate(good_id, Version::Unknown(String::new())))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Verification { .. }), "got {err}");
}

// Pip

pub struct PipBackend;

impl PipBackend {
    pub fn new() -> Self {
        Self
    }

    /// How to start pip so that its JSON arrives as UTF-8.
    ///
    /// On Windows a redirected `pip inspect` writes in the ANSI code page and
    /// crashes on the first package description outside it. `python -X utf8`
    /// avoids that, but is used only when it is the same pip: the first
    /// `python` on PATH may belong to another installation.
    async fn command() -> (&'static str, &'static [&'static str]) {
        const PYTHON: &str = if cfg!(windows) { "python" } else { "python3" };
        const UTF8_PIP: &[&str] = &["-X", "utf8", "-m", "pip"];
        let plain = proc::run("pip", &["--version"], QUERY_TIMEOUT).await;
        let python = proc::run(
            PYTHON,
            &["-X", "utf8", "-m", "pip", "--version"],
            QUERY_TIMEOUT,
        )
        .await;
        match (plain, python) {
            (Ok(a), Ok(b)) if b.success() && a.stdout.trim() == b.stdout.trim() => {
                (PYTHON, UTF8_PIP)
            }
            _ => ("pip", &[]),
        }
    }

    async fn run(
        (program, prefix): (&str, &[&str]),
        args: &[&str],
        timeout: Duration,
    ) -> Result<proc::Output> {
        let all: Vec<&str> = prefix.iter().chain(args).copied().collect();
        proc::run(program, &all, timeout).await
    }

    /// Refuse when installing `spec` would leave another package without the
    /// version it needs.
    ///
    /// pip resolves only what it is asked for. What else is installed, and
    /// which versions that still needs, it does not look at; that is how
    /// updating packages one by one breaks a Python installation.
    async fn refuse_if_it_breaks_others(
        pip: (&str, &[&str]),
        candidate: &UpdateCandidate,
        spec: &str,
    ) -> Result<()> {
        let plan = Self::run(
            pip,
            &[
                "install",
                "--upgrade",
                "--no-input",
                "--disable-pip-version-check",
                "--dry-run",
                "--quiet",
                "--report",
                "-",
                spec,
            ],
            INSTALL_TIMEOUT,
        )
        .await?;
        if !plan.success() {
            // Both options came with pip 22.2. Without them `pip check` after
            // the installation is what is left.
            if plan.stderr.contains("no such option") {
                return Ok(());
            }
            return Err(pip_failed(format!("pip install --dry-run {spec}"), plan));
        }
        let changes = parse_pip_report(&plan.stdout)?;

        let inspect = Self::run(
            pip,
            &["inspect", "--disable-pip-version-check"],
            QUERY_TIMEOUT,
        )
        .await?;
        if !inspect.success() {
            return Err(guard::failed("pip inspect", inspect));
        }
        let environment = parse_pip_inspect(&inspect.stdout)?;
        let problems = environment.problems_with(&candidate.id.native, &changes);
        if problems.is_empty() {
            Ok(())
        } else {
            Err(guard::refused(
                candidate,
                format!(
                    "this update would break other packages: {}",
                    problems.join("; ")
                ),
            ))
        }
    }

    /// The broken requirements `pip check` finds, one line each.
    async fn broken_requirements(pip: (&str, &[&str])) -> Result<Vec<String>> {
        let out = Self::run(
            pip,
            &["check", "--disable-pip-version-check"],
            QUERY_TIMEOUT,
        )
        .await?;
        Ok(parse_pip_check(&out.stdout))
    }
}

impl Default for PipBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for PipBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Pip
    }
    fn display_name(&self) -> &str {
        "Python pip"
    }

    async fn is_available(&self) -> bool {
        proc::exists("pip", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        // Only what nothing else depends on. A dependency has the version the
        // packages above it ask for, and moves when they are updated.
        let out = proc::run(
            "pip",
            &[
                "list",
                "--outdated",
                "--not-required",
                "--format=json",
                "--disable-pip-version-check",
            ],
            SCAN_TIMEOUT,
        )
        .await?;
        if !out.success() {
            return Err(guard::failed("pip list --outdated", out));
        }
        parse_pip_outdated(&out.stdout)
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        guard::precheck(candidate)?;
        if let Some(reason) = pip_self_update_refusal(&candidate.id.native, cfg!(windows)) {
            return Err(guard::refused(candidate, reason));
        }
        guard::still_older(
            candidate,
            self.installed_version(candidate).await?.as_deref(),
        )?;

        // The version must be attached to the name as `name==version` in a
        // single argument. Passing it as a separate arg made pip read it as a
        // second package to install (literally named "1.5.1"), so every pip
        // upgrade failed, the cause of a machine-full of "failed" history
        // entries.
        let spec = pip_install_spec(&candidate.id.native, candidate.available.raw());
        let pip = Self::command().await;
        Self::refuse_if_it_breaks_others(pip, candidate, &spec).await?;
        let before = Self::broken_requirements(pip).await?;

        let out = Self::run(
            pip,
            &[
                "install",
                "--upgrade",
                "--no-input",
                "--disable-pip-version-check",
                &spec,
            ],
            INSTALL_TIMEOUT,
        )
        .await?;
        if !out.success() {
            return Err(pip_failed(format!("pip install --upgrade {spec}"), out));
        }

        let after = Self::broken_requirements(pip).await?;
        let new = newly_broken(&before, &after);
        if new.is_empty() {
            Ok(())
        } else {
            Err(guard::refused(
                candidate,
                format!("installed, but `pip check` now reports: {}", new.join("; ")),
            ))
        }
    }

    async fn list_installed(&self) -> Result<Vec<InstalledPackage>> {
        let out = proc::run("pip", &["list", "--format=json"], QUERY_TIMEOUT).await?;
        parse_pip_list(&out.stdout)
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let out = proc::run("pip", &["show", &candidate.id.native], QUERY_TIMEOUT).await?;
        for line in out.stdout.lines() {
            if let Some(v) = line.strip_prefix("Version:") {
                return Ok(Some(v.trim().to_string()));
            }
        }
        Ok(None)
    }
}

#[derive(serde::Deserialize)]
struct PipOutdated {
    name: String,
    version: String,
    latest_version: String,
}

#[derive(serde::Deserialize)]
struct PipInstalled {
    name: String,
    version: String,
}

/// Parse `pip list --outdated --format=json`.
fn parse_pip_outdated(stdout: &str) -> Result<Vec<UpdateCandidate>> {
    let packages: Vec<PipOutdated> = serde_json::from_str(stdout.trim())
        .map_err(|e| Error::parse("pip", format!("JSON parse: {e}")))?;
    Ok(packages
        .into_iter()
        .map(|p| UpdateCandidate {
            id: PackageId::new(BackendKind::Pip, p.name.as_str()),
            name: p.name,
            installed: Version::parse_as(&p.version, Scheme::Pep440),
            available: Version::parse_as(&p.latest_version, Scheme::Pep440),
            size_bytes: None,
            expected_sha256: None,
        })
        .collect())
}

/// Build the pinned requirement pip expects: `name==version`, one argument.
fn pip_install_spec(name: &str, version: &str) -> String {
    format!("{name}=={version}")
}

/// pip cannot replace its own launcher while it runs on Windows: it stops with
/// "To modify pip, please run the following command".
fn pip_self_update_refusal(id: &str, windows: bool) -> Option<&'static str> {
    (windows && id.eq_ignore_ascii_case("pip")).then_some(
        "pip cannot replace itself while it is running; update it with \
         `python -m pip install --upgrade pip` in a terminal",
    )
}

/// A failed pip, in plain words where the user has to act.
fn pip_failed(command: String, out: proc::Output) -> Error {
    match explain_pip_failure(&out.stderr) {
        Some(reason) => Error::unavailable("Python pip", reason),
        None => guard::failed(command, out),
    }
}

/// The two failures a user can only fix outside Odysync.
fn explain_pip_failure(stderr: &str) -> Option<&'static str> {
    if stderr.contains("externally-managed-environment") {
        // PEP 668. Overriding it can break the operating system's own tools,
        // so Odysync never passes --break-system-packages.
        return Some(
            "this Python installation is managed by the operating system (PEP 668), so pip \
             may not change it; update the package with the system's package manager or use \
             a virtual environment",
        );
    }
    if stderr.contains("To modify pip, please run the following command") {
        return Some(
            "pip cannot replace itself while it is running; update it with \
             `python -m pip install --upgrade pip` in a terminal",
        );
    }
    None
}

/// Parse `pip list --format=json`: `[{"name": "...", "version": "..."}]`.
fn parse_pip_list(stdout: &str) -> Result<Vec<InstalledPackage>> {
    let packages: Vec<PipInstalled> = serde_json::from_str(stdout)
        .map_err(|e| Error::parse("pip", format!("JSON parse: {e}")))?;
    Ok(packages
        .into_iter()
        .map(|p| InstalledPackage {
            id: PackageId::new(BackendKind::Pip, &p.name),
            name: p.name,
            version: p.version,
        })
        .collect())
}

/// The broken requirements in `pip check`'s output. It writes one sentence per
/// problem and "No broken requirements found." when there is none.
fn parse_pip_check(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| l.contains(" has requirement ") || l.contains(" requires "))
        .map(str::to_string)
        .collect()
}

/// Problems that were not there before.
fn newly_broken(before: &[String], after: &[String]) -> Vec<String> {
    after
        .iter()
        .filter(|l| !before.contains(l))
        .cloned()
        .collect()
}

/// `Foo.Bar_baz` and `foo-bar-baz` are the same package (PEP 503).
fn pip_normalize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.trim().chars() {
        if matches!(c, '-' | '_' | '.') {
            if !out.ends_with('-') {
                out.push('-');
            }
        } else {
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}

#[derive(serde::Deserialize)]
struct PipMetadata {
    name: String,
    version: String,
    #[serde(default)]
    requires_dist: Vec<String>,
}

#[derive(serde::Deserialize)]
struct PipDistribution {
    metadata: PipMetadata,
}

/// What a dry run would install: name and version.
///
/// <https://pip.pypa.io/en/stable/reference/installation-report/>
fn parse_pip_report(stdout: &str) -> Result<Vec<(String, String)>> {
    #[derive(serde::Deserialize)]
    struct Report {
        install: Vec<PipDistribution>,
    }
    let report: Report = serde_json::from_str(stdout.trim())
        .map_err(|e| Error::parse("pip installation report", e.to_string()))?;
    Ok(report
        .install
        .into_iter()
        .map(|d| (d.metadata.name, d.metadata.version))
        .collect())
}

/// What is installed, with the requirements each package declares.
struct PipEnvironment {
    installed: Vec<PipMetadata>,
    markers: HashMap<String, String>,
}

/// <https://pip.pypa.io/en/stable/reference/inspect-report/>
fn parse_pip_inspect(stdout: &str) -> Result<PipEnvironment> {
    #[derive(serde::Deserialize)]
    struct Inspect {
        installed: Vec<PipDistribution>,
        #[serde(default)]
        environment: HashMap<String, String>,
    }
    let parsed: Inspect = serde_json::from_str(stdout.trim())
        .map_err(|e| Error::parse("pip inspect", e.to_string()))?;
    Ok(PipEnvironment {
        installed: parsed.installed.into_iter().map(|d| d.metadata).collect(),
        markers: parsed.environment,
    })
}

impl PipEnvironment {
    /// What installing `changes` would do to packages other than `target`.
    fn problems_with(&self, target: &str, changes: &[(String, String)]) -> Vec<String> {
        let target = pip_normalize(target);
        let before: HashMap<String, &str> = self
            .installed
            .iter()
            .map(|p| (pip_normalize(&p.name), p.version.as_str()))
            .collect();
        let after: HashMap<String, &str> = changes
            .iter()
            .map(|(n, v)| (pip_normalize(n), v.as_str()))
            .collect();

        let mut problems = Vec::new();
        for (name, new) in &after {
            if let Some(old) = before.get(name)
                && *name != target
                && Scheme::Pep440.compare(old, new) == Some(std::cmp::Ordering::Greater)
            {
                problems.push(format!("{name} would go back from {old} to {new}"));
            }
        }
        for package in &self.installed {
            // A package that is replaced brings its own requirements, and pip
            // has already solved those.
            if after.contains_key(&pip_normalize(&package.name)) {
                continue;
            }
            for line in &package.requires_dist {
                let Some(need) = PipRequirement::parse(line) else {
                    continue;
                };
                let Some(new) = after.get(&need.name) else {
                    continue;
                };
                if !need.applies(&self.markers) || need.accepts(new) {
                    continue;
                }
                // Broken before is not broken by this update.
                if before.get(&need.name).is_some_and(|old| !need.accepts(old)) {
                    continue;
                }
                problems.push(format!(
                    "{} needs {}{} but would get {new}",
                    package.name, need.name, need.specifier
                ));
            }
        }
        problems.sort();
        problems
    }
}

/// One `requires_dist` entry (PEP 508), as far as it is needed here.
#[derive(Debug)]
struct PipRequirement {
    name: String,
    specifier: String,
    marker: String,
}

impl PipRequirement {
    fn parse(line: &str) -> Option<Self> {
        let (requirement, marker) = line.split_once(';').unwrap_or((line, ""));
        // `name @ https://...` pins a file, not a version.
        if requirement.contains('@') {
            return None;
        }
        let requirement = requirement.trim();
        let end = requirement
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
            .unwrap_or(requirement.len());
        let (name, mut rest) = requirement.split_at(end);
        rest = rest.trim();
        if let Some(extras) = rest.strip_prefix('[') {
            rest = extras.split_once(']')?.1.trim();
        }
        let specifier = rest.trim_matches(|c| c == '(' || c == ')').trim();
        if name.is_empty() {
            return None;
        }
        Some(Self {
            name: pip_normalize(name),
            specifier: specifier.to_string(),
            marker: marker.trim().to_string(),
        })
    }

    /// Whether the requirement holds on this machine. What cannot be decided
    /// counts as "yes": the result is at worst a refused update.
    fn applies(&self, markers: &HashMap<String, String>) -> bool {
        // `pip check` does not look at optional extras either.
        if self.marker.contains("extra") {
            return false;
        }
        if self.marker.is_empty() || self.marker.contains(" or ") || self.marker.contains('(') {
            return true;
        }
        self.marker.split(" and ").all(|clause| {
            let clause = clause.trim();
            for op in ["==", "!=", "<=", ">=", "<", ">"] {
                let Some((left, right)) = clause.split_once(op) else {
                    continue;
                };
                let wanted = right.trim().trim_matches(['"', '\'']);
                let Some(actual) = markers.get(left.trim()) else {
                    return true;
                };
                return if left.trim().starts_with("python_") {
                    pep440_matches(actual, op, wanted).unwrap_or(true)
                } else {
                    match op {
                        "==" => actual == wanted,
                        "!=" => actual != wanted,
                        _ => true,
                    }
                };
            }
            true
        })
    }

    /// Whether `version` satisfies the specifier. What cannot be read counts
    /// as "no", for the same reason.
    fn accepts(&self, version: &str) -> bool {
        self.specifier
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .all(|clause| {
                ["===", "~=", "==", "!=", "<=", ">=", "<", ">"]
                    .iter()
                    .find_map(|op| Some((*op, clause.strip_prefix(op)?.trim())))
                    .and_then(|(op, wanted)| pep440_matches(version, op, wanted))
                    .unwrap_or(false)
            })
    }
}

/// One version specifier clause of PEP 440.
fn pep440_matches(version: &str, op: &str, wanted: &str) -> Option<bool> {
    use std::cmp::Ordering::{Equal, Greater, Less};

    // `1.4.*`: the release starts with these numbers.
    let has_prefix = |prefix: &str| -> Option<bool> {
        let numbers = |v: &str| -> Vec<u64> {
            let v = v.split(['+', '!']).next().unwrap_or("");
            v.split('.')
                .map_while(|s| {
                    let digits: String = s.chars().take_while(char::is_ascii_digit).collect();
                    digits.parse().ok()
                })
                .collect()
        };
        let wanted = numbers(prefix);
        if wanted.is_empty() {
            return None;
        }
        let mut have = numbers(version);
        have.resize(have.len().max(wanted.len()), 0);
        Some(have[..wanted.len()] == wanted[..])
    };
    let public = |v: &str| -> String {
        // A local version (`2.1+cu121`) equals the public one unless the
        // specifier names a local part itself.
        if wanted.contains('+') {
            v.to_string()
        } else {
            v.split('+').next().unwrap_or(v).to_string()
        }
    };
    let order = |to: &str| Scheme::Pep440.compare(&public(version), to);

    match op {
        "===" => Some(version == wanted),
        "==" | "!=" => {
            let equal = match wanted.strip_suffix(".*") {
                Some(prefix) => has_prefix(prefix)?,
                None => order(wanted)? == Equal,
            };
            Some(equal == (op == "=="))
        }
        "~=" => {
            let (prefix, _) = wanted.rsplit_once('.')?;
            Some(order(wanted)? != Less && has_prefix(prefix)?)
        }
        "<" => Some(order(wanted)? == Less),
        "<=" => Some(order(wanted)? != Greater),
        ">" => Some(order(wanted)? == Greater),
        ">=" => Some(order(wanted)? != Less),
        _ => None,
    }
}

// Cargo

pub struct CargoBackend;

impl CargoBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CargoBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for CargoBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Cargo
    }
    fn display_name(&self) -> &str {
        "Rust cargo"
    }

    async fn is_available(&self) -> bool {
        proc::exists("cargo", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let out = proc::run("cargo", &["install", "--list"], QUERY_TIMEOUT).await?;
        if !out.success() {
            return Err(guard::failed("cargo install --list", out));
        }
        let custom = custom_builds()?;
        let crates: Vec<(String, String)> = parse_crates_io_installs(&out.stdout)
            .into_iter()
            .filter(|(name, _)| !custom.contains(name))
            .collect();
        if crates.is_empty() {
            return Ok(Vec::new());
        }

        let client = crate::direct::http::client(None, QUERY_TIMEOUT)?;
        let mut found = Vec::new();
        let mut last_error = None;
        for (name, installed) in &crates {
            let Some(url) = crates_io_index_url(name) else {
                continue;
            };
            match crate::direct::http::get_text(&client, &url).await {
                Ok(body) => {
                    if let Some(latest) = newest_published(&body)
                        && latest != *installed
                    {
                        found.push(UpdateCandidate {
                            id: PackageId::new(BackendKind::Cargo, name.as_str()),
                            name: name.clone(),
                            installed: Version::parse(installed),
                            available: Version::parse(&latest),
                            size_bytes: None,
                            expected_sha256: None,
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!(krate = %name, error = %e, "crates.io index lookup failed");
                    last_error = Some(e);
                }
            }
        }
        match last_error {
            Some(e) if found.is_empty() => Err(e),
            _ => Ok(found),
        }
    }

    async fn list_installed(&self) -> Result<Vec<InstalledPackage>> {
        let out = proc::run("cargo", &["install", "--list"], QUERY_TIMEOUT).await?;
        Ok(parse_cargo_install_list(&out.stdout))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        guard::precheck(candidate)?;
        if custom_builds()?.contains(&candidate.id.native) {
            return Err(guard::refused(
                candidate,
                "this crate was installed with its own features or profile, which \
                 `cargo install` would silently drop; update it by hand",
            ));
        }
        // Only crates from crates.io: a path or git install must never be
        // replaced by a published crate of the same name.
        let out = proc::run("cargo", &["install", "--list"], QUERY_TIMEOUT).await?;
        let current = parse_crates_io_installs(&out.stdout)
            .into_iter()
            .find(|(name, _)| *name == candidate.id.native)
            .map(|(_, version)| version);
        guard::still_older(candidate, current.as_deref())?;

        let out = proc::run(
            "cargo",
            &[
                "install",
                "--version",
                candidate.available.raw(),
                "--",
                &candidate.id.native,
            ],
            COMPILE_TIMEOUT,
        )
        .await?;
        if out.success() {
            Ok(())
        } else {
            Err(guard::failed(
                format!("cargo install {}", candidate.id.native),
                out,
            ))
        }
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        let out = proc::run("cargo", &["install", "--list"], QUERY_TIMEOUT).await?;
        Ok(parse_cargo_install_list(&out.stdout)
            .into_iter()
            .find(|p| p.id.native == candidate.id.native)
            .map(|p| p.version))
    }
}

/// Parse `cargo install --list`.
///
/// ```text
/// ripgrep v14.1.0:
///     rg
/// cargo-edit v0.12.2 (/home/u/src/cargo-edit):
///     cargo-add
/// ```
///
/// Crate lines start at column 0; the binaries each crate installs are indented
/// beneath it and must not be mistaken for packages.
fn parse_cargo_install_list(stdout: &str) -> Vec<InstalledPackage> {
    stdout
        .lines()
        .filter(|line| !line.starts_with(char::is_whitespace))
        .filter_map(|line| {
            let (name, rest) = line.trim_end().split_once(' ')?;
            // The version token is `vX.Y.Z`; a path-installed crate appends the
            // source directory in parentheses, which is not part of the version.
            let version = rest.strip_prefix('v')?.split_whitespace().next()?;
            let version = version.trim_end_matches(':');
            if name.is_empty() || version.is_empty() {
                return None;
            }
            Some(InstalledPackage {
                id: PackageId::new(BackendKind::Cargo, name),
                name: name.to_string(),
                version: version.to_string(),
            })
        })
        .collect()
}

/// Name and version of the crates that came from crates.io. Cargo writes the
/// source of everything else (a path, a git URL, another registry) behind the
/// version in parentheses.
fn parse_crates_io_installs(stdout: &str) -> Vec<(String, String)> {
    let from_elsewhere = |line: &&str| line.contains('(');
    stdout
        .lines()
        .filter(|line| !from_elsewhere(line))
        .flat_map(parse_cargo_install_list)
        .map(|p| (p.id.native, p.version))
        .collect()
}

/// Where crates.io's sparse index keeps a crate, or `None` for something that
/// is not a crate name.
///
/// <https://doc.rust-lang.org/cargo/reference/registry-index.html#index-files>
fn crates_io_index_url(name: &str) -> Option<String> {
    let ok = name.bytes().next().is_some_and(|b| b.is_ascii_alphabetic())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !ok {
        return None;
    }
    let n = name.to_ascii_lowercase();
    let path = match n.len() {
        1 => format!("1/{n}"),
        2 => format!("2/{n}"),
        3 => format!("3/{}/{n}", &n[..1]),
        _ => format!("{}/{}/{n}", &n[..2], &n[2..4]),
    };
    Some(format!("https://index.crates.io/{path}"))
}

/// The newest release in an index file that is neither yanked nor a
/// pre-release. One JSON object per line.
fn newest_published(body: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Release {
        vers: String,
        #[serde(default)]
        yanked: bool,
    }
    let releases: Vec<String> = body
        .lines()
        .filter_map(|line| serde_json::from_str::<Release>(line.trim()).ok())
        .filter(|r| !r.yanked)
        // A semver pre-release does not have to use one of the known words, so
        // the hyphen decides, not the name.
        .filter(|r| !r.vers.split('+').next().unwrap_or("").contains('-'))
        .map(|r| r.vers)
        .collect();
    guard::newest(releases.iter().map(String::as_str), Version::parse).map(str::to_string)
}

/// Crates on this machine that were not built the default way.
fn custom_builds() -> Result<HashSet<String>> {
    let home = std::env::var_os("CARGO_INSTALL_ROOT")
        .or_else(|| std::env::var_os("CARGO_HOME"))
        .map(std::path::PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().join(".cargo")));
    let Some(file) = home.map(|h| h.join(".crates2.json")) else {
        return Ok(HashSet::new());
    };
    match std::fs::read_to_string(&file) {
        Ok(text) => parse_custom_builds(&text),
        // Cargo writes this file with every install. Without it there is
        // nothing that was installed in a special way.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashSet::new()),
        Err(e) => Err(e.into()),
    }
}

/// Read `.crates2.json`: which crates carry features or a profile of their own.
fn parse_custom_builds(json: &str) -> Result<HashSet<String>> {
    #[derive(serde::Deserialize)]
    struct File {
        #[serde(default)]
        installs: HashMap<String, Install>,
    }
    #[derive(serde::Deserialize)]
    struct Install {
        #[serde(default)]
        features: Vec<String>,
        #[serde(default)]
        all_features: bool,
        #[serde(default)]
        no_default_features: bool,
        #[serde(default)]
        profile: Option<String>,
    }
    let file: File = serde_json::from_str(json)
        .map_err(|e| Error::parse(".crates2.json", format!("JSON parse: {e}")))?;
    Ok(file
        .installs
        .into_iter()
        .filter(|(_, i)| {
            !i.features.is_empty()
                || i.all_features
                || i.no_default_features
                || i.profile.as_deref().is_some_and(|p| p != "release")
        })
        .filter_map(|(key, _)| key.split_whitespace().next().map(str::to_string))
        .collect())
}

// Npm

pub struct NpmBackend;

impl NpmBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NpmBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for NpmBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Npm
    }
    fn display_name(&self) -> &str {
        "Node.js npm (global)"
    }

    async fn is_available(&self) -> bool {
        proc::exists("npm", &["--version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        let out = proc::run("npm", &["outdated", "-g", "--json"], SCAN_TIMEOUT).await?;
        // npm exits with 1 both when there are updates and when it failed, so
        // only the output tells the two apart.
        if out.stdout.trim().is_empty() && !out.success() {
            return Err(guard::failed("npm outdated -g", out));
        }
        parse_npm_outdated(&out.stdout)
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        guard::precheck(candidate)?;
        guard::still_older(
            candidate,
            self.installed_version(candidate).await?.as_deref(),
        )?;
        let pkg = format!("{}@{}", candidate.id.native, candidate.available.raw());
        let out = proc::run("npm", &["install", "-g", "--", &pkg], INSTALL_TIMEOUT).await?;
        if out.success() {
            Ok(())
        } else {
            Err(guard::failed(format!("npm install -g {pkg}"), out))
        }
    }

    async fn list_installed(&self) -> Result<Vec<InstalledPackage>> {
        let out = proc::run("npm", &["list", "-g", "--depth=0", "--json"], QUERY_TIMEOUT).await?;
        parse_npm_global_list(&out.stdout)
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        Ok(self
            .list_installed()
            .await?
            .into_iter()
            .find(|p| p.id.native == candidate.id.native)
            .map(|p| p.version))
    }
}

/// Parse `npm outdated -g --json`: an object keyed by package name.
fn parse_npm_outdated(stdout: &str) -> Result<Vec<UpdateCandidate>> {
    #[derive(serde::Deserialize)]
    struct Outdated {
        current: Option<String>,
        wanted: Option<String>,
        latest: Option<String>,
    }
    if stdout.trim().is_empty() {
        return Ok(Vec::new());
    }
    let parsed: serde_json::Map<String, serde_json::Value> = serde_json::from_str(stdout.trim())
        .map_err(|e| Error::parse("npm outdated", format!("JSON parse: {e}")))?;
    npm_error(&parsed, "npm outdated")?;

    let mut found = Vec::new();
    for (name, value) in parsed {
        let Ok(entry) = serde_json::from_value::<Outdated>(value) else {
            continue;
        };
        // A linked or git package is the user's own copy; installing the
        // registry's package of that name would replace it.
        if matches!(
            entry.wanted.as_deref(),
            Some("linked" | "git" | "remote" | "file")
        ) {
            continue;
        }
        // Without `current` the package is not installed.
        let (Some(current), Some(latest)) = (entry.current, entry.latest) else {
            continue;
        };
        found.push(UpdateCandidate {
            id: PackageId::new(BackendKind::Npm, name.as_str()),
            name,
            installed: Version::parse(&current),
            available: Version::parse(&latest),
            size_bytes: None,
            expected_sha256: None,
        });
    }
    Ok(found)
}

/// npm reports its own failures as `{"error": {"code": ..., "summary": ...}}`.
fn npm_error(parsed: &serde_json::Map<String, serde_json::Value>, what: &str) -> Result<()> {
    let Some(error) = parsed.get("error").and_then(|e| e.as_object()) else {
        return Ok(());
    };
    // A package may well be called "error"; its entry carries a version.
    if error.contains_key("current") || error.contains_key("latest") {
        return Ok(());
    }
    let text = |key: &str| error.get(key).and_then(|v| v.as_str()).unwrap_or("");
    Err(Error::CommandFailed {
        command: what.to_string(),
        code: 1,
        stderr: format!("{} {}", text("code"), text("summary"))
            .trim()
            .to_string(),
    })
}

/// Parse `npm list -g --depth=0 --json`.
///
/// ```json
/// {"name": "lib", "dependencies": {"npm": {"version": "10.5.0"}}}
/// ```
///
/// Entries npm marks as missing carry no `version`; those are skipped rather
/// than reported with an empty version string. The name is looked up as a key,
/// not as a JSON pointer: the slash in `@scope/name` is a path separator there
/// and no scoped package was ever found.
fn parse_npm_global_list(stdout: &str) -> Result<Vec<InstalledPackage>> {
    let parsed: serde_json::Map<String, serde_json::Value> = serde_json::from_str(stdout.trim())
        .map_err(|e| Error::parse("npm list", format!("JSON parse: {e}")))?;
    // `npm list` sets "error" next to a complete list when a dependency of a
    // package is wrong, so only a list without dependencies is a failure.
    let Some(deps) = parsed.get("dependencies").and_then(|d| d.as_object()) else {
        npm_error(&parsed, "npm list")?;
        return Ok(Vec::new());
    };
    Ok(deps
        .iter()
        .filter_map(|(name, entry)| {
            let version = entry.get("version")?.as_str()?;
            Some(InstalledPackage {
                id: PackageId::new(BackendKind::Npm, name.as_str()),
                name: name.clone(),
                version: version.to_string(),
            })
        })
        .collect())
}

// Go

pub struct GoBackend;

impl GoBackend {
    pub fn new() -> Self {
        Self
    }

    /// The tools in the directory `go install` writes to.
    async fn installed_tools() -> Result<Vec<GoTool>> {
        let out = proc::run("go", &["env", "-json", "GOBIN", "GOPATH"], QUERY_TIMEOUT).await?;
        if !out.success() {
            return Err(guard::failed("go env", out));
        }
        let Some(dir) = go_bin_dir(&out.stdout)? else {
            return Ok(Vec::new());
        };
        if !std::path::Path::new(&dir).is_dir() {
            return Ok(Vec::new());
        }
        let out = proc::run("go", &["version", "-m", &dir], QUERY_TIMEOUT).await?;
        if !out.success() {
            return Err(guard::failed("go version -m", out));
        }
        Ok(parse_go_version_m(&out.stdout))
    }
}

impl Default for GoBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for GoBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Go
    }
    fn display_name(&self) -> &str {
        "Go tools"
    }

    async fn is_available(&self) -> bool {
        proc::exists("go", &["version"]).await
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        // Only tools, never the module in whatever directory Odysync happens
        // to run in: `go list -m -u all` listed that project's dependencies,
        // and `go install` cannot update those.
        let tools = Self::installed_tools().await?;
        // `-C` takes the command out of any module around the current
        // directory. https://pkg.go.dev/cmd/go: "If used, the -C flag must be
        // the first one on the command line."
        let outside = std::env::temp_dir().to_string_lossy().into_owned();
        let mut found = Vec::new();
        let mut last_error = None;
        for tool in tools {
            let query = format!("{}@latest", tool.module);
            let out = proc::run(
                "go",
                &["list", "-C", &outside, "-m", "-json", &query],
                QUERY_TIMEOUT,
            )
            .await?;
            if !out.success() {
                let e = guard::failed(format!("go list -m {query}"), out);
                tracing::warn!(error = %e, "could not look up the newest version");
                last_error = Some(e);
                continue;
            }
            let Some(latest) = parse_go_module_version(&out.stdout) else {
                continue;
            };
            if latest == tool.version {
                continue;
            }
            found.push(UpdateCandidate {
                name: tool.path.clone(),
                id: PackageId::new(BackendKind::Go, tool.path),
                installed: Version::parse(&tool.version),
                available: Version::parse(&latest),
                size_bytes: None,
                expected_sha256: None,
            });
        }
        match last_error {
            Some(e) if found.is_empty() => Err(e),
            _ => Ok(found),
        }
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        guard::precheck(candidate)?;
        guard::still_older(
            candidate,
            self.installed_version(candidate).await?.as_deref(),
        )?;
        // `go install pkg@version` is the supported way to install a pinned
        // tool. `go get -u` upgraded to *latest* (ignoring the target version)
        // and is deprecated for installing executables outside a module.
        let spec = format!("{}@{}", candidate.id.native, candidate.available.raw());
        let out = proc::run("go", &["install", "--", &spec], COMPILE_TIMEOUT).await?;
        if out.success() {
            Ok(())
        } else {
            Err(guard::failed(format!("go install {spec}"), out))
        }
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        Ok(Self::installed_tools()
            .await?
            .into_iter()
            .find(|t| t.path == candidate.id.native)
            .map(|t| t.version))
    }
}

/// One binary built from a published module.
#[derive(Debug, PartialEq, Eq)]
struct GoTool {
    /// Package path, which is what `go install` takes.
    path: String,
    /// Module the package belongs to, which is what has versions.
    module: String,
    version: String,
}

/// `GOBIN`, or `bin` in the first `GOPATH` entry.
fn go_bin_dir(json: &str) -> Result<Option<String>> {
    let env: HashMap<String, String> = serde_json::from_str(json.trim())
        .map_err(|e| Error::parse("go env", format!("JSON parse: {e}")))?;
    let value = |key: &str| env.get(key).filter(|v| !v.trim().is_empty());
    if let Some(bin) = value("GOBIN") {
        return Ok(Some(bin.clone()));
    }
    Ok(value("GOPATH").and_then(|paths| {
        let first = std::env::split_paths(paths).next()?;
        Some(first.join("bin").to_string_lossy().into_owned())
    }))
}

/// Parse `go version -m <dir>`.
///
/// A binary that was built from a local checkout says `(devel)`, one whose
/// module was replaced has a `=>` line; neither is what the module proxy
/// serves under that name, so neither is offered.
fn parse_go_version_m(stdout: &str) -> Vec<GoTool> {
    let mut tools: Vec<GoTool> = Vec::new();
    let mut path: Option<String> = None;
    // Whether the last line was the `mod` line of an accepted tool.
    let mut after_mod = false;
    for line in stdout.lines() {
        if !line.starts_with(char::is_whitespace) {
            path = None;
            after_mod = false;
            continue;
        }
        let mut fields = line.split_whitespace();
        match (fields.next(), fields.next(), fields.next()) {
            (Some("path"), Some(p), _) => path = Some(p.to_string()),
            (Some("mod"), Some(module), Some(version)) => {
                let published = version.starts_with('v') && !version.contains("+dirty");
                if let Some(path) = path.take()
                    && published
                    && path.starts_with(module)
                    && !tools.iter().any(|t| t.path == path)
                {
                    tools.push(GoTool {
                        path,
                        module: module.to_string(),
                        version: version.to_string(),
                    });
                    after_mod = true;
                    continue;
                }
            }
            (Some("=>"), ..) if after_mod => {
                tools.pop();
            }
            _ => {}
        }
        after_mod = false;
    }
    tools
}

/// `Version` in the object `go list -m -json module@latest` prints.
fn parse_go_module_version(json: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(json.trim()).ok()?;
    let version = parsed.get("Version")?.as_str()?;
    version.starts_with('v').then(|| version.to_string())
}

// .NET tools, VS Code extensions, JetBrains plugins

// PowerShell modules

pub struct PowerShellModuleBackend;

impl PowerShellModuleBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PowerShellModuleBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// PowerShell 7 when it is there, Windows PowerShell otherwise.
async fn powershell_program() -> &'static str {
    if proc::exists("pwsh", &["--version"]).await {
        "pwsh"
    } else {
        "powershell"
    }
}

#[async_trait]
impl Backend for PowerShellModuleBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::PowerShellModule
    }
    fn display_name(&self) -> &str {
        "PowerShell modules"
    }

    async fn is_available(&self) -> bool {
        // Short-circuit on the cheap cfg check before spawning anything.
        cfg!(windows)
            && (proc::exists("pwsh", &["--version"]).await
                || proc::exists("powershell", &["-Command", "$PSVersionTable.PSVersion"]).await)
    }

    async fn scan(&self) -> Result<Vec<UpdateCandidate>> {
        // One PowerShell, one Gallery query for every module at once. Only
        // modules that came from the PowerShell Gallery are compared against
        // it: a module from a private repository must never be "updated" to a
        // public one that happens to share its name.
        let out = proc::run(
            powershell_program().await,
            &["-NoProfile", "-NonInteractive", "-Command", PS_MODULE_SCAN],
            SCAN_TIMEOUT,
        )
        .await?;
        Ok(parse_ps_modules(&out.stdout, self.kind()))
    }

    async fn apply(&self, candidate: &UpdateCandidate) -> Result<()> {
        guard::precheck(candidate)?;
        let command = update_module_command(&candidate.id.native, candidate.available.raw())?;
        guard::still_older(
            candidate,
            self.installed_version(candidate).await?.as_deref(),
        )?;
        let out = proc::run(
            powershell_program().await,
            &["-NoProfile", "-NonInteractive", "-Command", &command],
            INSTALL_TIMEOUT,
        )
        .await?;
        if out.success() {
            Ok(())
        } else {
            Err(guard::failed(
                format!("Update-Module {}", candidate.id.native),
                out,
            ))
        }
    }

    async fn installed_version(&self, candidate: &UpdateCandidate) -> Result<Option<String>> {
        if !is_module_name(&candidate.id.native) {
            return Ok(None);
        }
        let script = format!(
            "Get-InstalledModule -Name '{}' -AllVersions | Select-Object -ExpandProperty Version \
             | ForEach-Object {{ [string]$_ }}",
            candidate.id.native
        );
        let out = proc::run(
            powershell_program().await,
            &["-NoProfile", "-NonInteractive", "-Command", &script],
            QUERY_TIMEOUT,
        )
        .await?;
        // PowerShellGet returns versions as text, which PowerShell sorts by
        // letters: 2.9 above 2.10.
        let versions = out.stdout.lines().map(str::trim).filter(|v| !v.is_empty());
        Ok(guard::newest(versions, Version::parse).map(str::to_string))
    }
}

/// Installed Gallery modules with the Gallery's newest version, as JSON.
const PS_MODULE_SCAN: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
$installed = @(Get-InstalledModule | Where-Object Repository -eq 'PSGallery')
$latest = @{}
if ($installed.Count -gt 0) {
  Find-Module -Name ($installed.Name) -Repository PSGallery | ForEach-Object { $latest[$_.Name] = [string]$_.Version }
}
ConvertTo-Json -Compress -InputObject @($installed | ForEach-Object {
  [pscustomobject]@{ Name = $_.Name; Version = [string]$_.Version; Latest = $latest[$_.Name] }
})
"#;

/// One row of [`PS_MODULE_SCAN`]. PowerShell writes PascalCase keys; reading
/// them as lowercase made every scan parse to nothing.
#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PsModule {
    name: String,
    version: String,
    latest: Option<String>,
}

/// Module names as the Gallery allows them. Anything else is not put into a
/// PowerShell command line.
fn is_module_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The modules that have a newer version in the Gallery.
fn parse_ps_modules(json: &str, kind: BackendKind) -> Vec<UpdateCandidate> {
    let modules: Vec<PsModule> = serde_json::from_str(json.trim()).unwrap_or_default();
    modules
        .into_iter()
        .filter(|m| is_module_name(&m.name))
        .filter_map(|m| {
            let latest = m.latest.filter(|l| !l.is_empty() && *l != m.version)?;
            let name = m.name.clone();
            Some(UpdateCandidate {
                id: PackageId::new(kind, m.name),
                name,
                installed: Version::parse(&m.version),
                available: Version::parse(&latest),
                size_bytes: None,
                expected_sha256: None,
            })
        })
        .collect()
}

/// `Update-Module` to exactly the version that was approved, from the Gallery.
///
/// Without `-Force`: that switch reinstalls a module that is already there and
/// answers the question whether an untrusted repository may be used. Where the
/// user has not trusted the Gallery, the update fails and says so.
fn update_module_command(name: &str, version: &str) -> Result<String> {
    let version_ok = !version.is_empty()
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'));
    if !is_module_name(name) || !version_ok {
        return Err(Error::Verification {
            package: name.to_string(),
            detail: "refusing to build an Update-Module command from this name or version".into(),
        });
    }
    Ok(format!(
        "Update-Module -Name '{name}' -RequiredVersion '{version}' -Confirm:$false \
         -ErrorAction Stop"
    ))
}

// Sources that cannot work

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(kind: BackendKind, id: &str, from: &str, to: &str) -> UpdateCandidate {
        UpdateCandidate {
            id: PackageId::new(kind, id),
            name: id.into(),
            installed: Version::parse(from),
            available: Version::parse(to),
            size_bytes: None,
            expected_sha256: None,
        }
    }

    fn crlf(text: &str) -> String {
        text.replace('\n', "\r\n")
    }

    // The shared guard.

    #[test]
    fn a_moved_target_is_refused() {
        let c = candidate(BackendKind::Npm, "npm", "10.0.0", "11.0.0");
        assert!(guard::same_target(&c, Some("11.0.0")).is_ok());
        assert!(guard::same_target(&c, Some(" 11.0.0\r")).is_ok());
        for offered in [Some("11.0.1"), Some("10.9.0"), Some(""), None] {
            assert!(matches!(
                guard::same_target(&c, offered).unwrap_err(),
                Error::Verification { .. }
            ));
        }
    }

    #[test]
    fn a_package_that_is_gone_newer_or_unreadable_is_refused() {
        let c = candidate(BackendKind::Npm, "npm", "10.0.0", "11.0.0");
        assert!(guard::still_older(&c, Some("10.0.0")).is_ok());
        assert!(guard::still_older(&c, Some("10.5.0")).is_ok());
        for current in [Some("11.0.0"), Some("12.0.0"), Some("Unknown"), None] {
            assert!(matches!(
                guard::still_older(&c, current).unwrap_err(),
                Error::Verification { .. }
            ));
        }
    }

    #[test]
    fn the_installed_version_is_read_by_the_scheme_of_the_target() {
        // By the general rules this pair has no order; by dpkg's it does.
        let mut c = candidate(BackendKind::Apt, "openssl", "1", "2");
        c.available = Version::parse_as("1:1.1.1w-0+deb11u2", Scheme::Dpkg);
        assert!(guard::still_older(&c, Some("1:1.1.1w-0+deb11u1")).is_ok());
        assert!(guard::still_older(&c, Some("1:1.1.1w-0+deb11u3")).is_err());
    }

    #[test]
    fn versions_that_look_like_options_are_refused() {
        let mut c = candidate(BackendKind::Npm, "npm", "1.0", "2.0");
        assert!(guard::precheck(&c).is_ok());
        c.available = Version::parse("-1.0");
        assert!(guard::precheck(&c).is_err());
        c.available = Version::parse("1.0\u{0}");
        assert!(guard::precheck(&c).is_err());
    }

    #[test]
    fn the_newest_of_several_versions() {
        let v = |list: &[&'static str]| guard::newest(list.iter().copied(), Version::parse);
        assert_eq!(v(&["1.9", "1.10", "1.2"]), Some("1.10"));
        assert_eq!(v(&[]), None);
        assert_eq!(v(&["1.0"]), Some("1.0"));
        // Two that cannot be ordered: no guess.
        assert_eq!(v(&["1.0", "Unknown"]), None);
    }

    #[tokio::test]
    async fn every_tool_refuses_options_as_ids() {
        assert_refuses_bad_input(&PipBackend::new(), "requests", Some(Scheme::Pep440)).await;
        assert_refuses_bad_input(&NpmBackend::new(), "@scope/name", None).await;
        assert_refuses_bad_input(&CargoBackend::new(), "ripgrep", None).await;
        assert_refuses_bad_input(&GoBackend::new(), "golang.org/x/tools/gopls", None).await;
        assert_refuses_bad_input(&PowerShellModuleBackend::new(), "Pester", None).await;
    }

    // PowerShell.

    #[test]
    fn powershell_modules_with_a_newer_gallery_version_become_candidates() {
        let json = r#"[
          {"Name":"PSReadLine","Version":"2.3.5","Latest":"2.4.1"},
          {"Name":"Pester","Version":"5.7.1","Latest":"5.7.1"},
          {"Name":"Az.Accounts","Version":"4.0.2","Latest":null}
        ]"#;
        let found = parse_ps_modules(json, BackendKind::PowerShellModule);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "PSReadLine");
        assert_eq!(found[0].available.raw(), "2.4.1");
        assert!(found[0].installed.is_upgrade_to(&found[0].available));
    }

    #[test]
    fn a_single_module_and_garbage_both_parse() {
        // ConvertTo-Json -InputObject @(...) always writes an array, but a
        // broken PowerShell must not turn into a panic either.
        assert!(parse_ps_modules("", BackendKind::PowerShellModule).is_empty());
        assert!(parse_ps_modules("not json", BackendKind::PowerShellModule).is_empty());
    }

    #[test]
    fn update_module_installs_the_exact_version_and_refuses_odd_input() {
        let command = update_module_command("PSReadLine", "2.4.1").unwrap();
        assert_eq!(
            command,
            "Update-Module -Name 'PSReadLine' -RequiredVersion '2.4.1' -Confirm:$false \
             -ErrorAction Stop"
        );
        assert!(!command.contains("-Force"));
        assert!(update_module_command("x'; Remove-Item C:\\", "1.0").is_err());
        assert!(update_module_command("PSReadLine", "1.0; calc").is_err());
        assert!(update_module_command("PSReadLine", "").is_err());
    }

    // pip.

    #[test]
    fn pip_pins_the_version_in_one_argument() {
        // Regression: passing name and version as separate args made pip try to
        // install a second package literally named "1.5.1", failing every
        // upgrade. The pin must be a single `name==version` token.
        assert_eq!(pip_install_spec("yfinance", "1.5.1"), "yfinance==1.5.1");
    }

    /// Captured from pip 26.3 on Windows with `--not-required`.
    const PIP_OUTDATED: &str = r#"[{"name": "httpx2", "version": "2.16.0", "latest_version": "2.16.2", "latest_filetype": "wheel"}, {"name": "playwright", "version": "1.64.1", "latest_version": "1.65.2", "latest_filetype": "wheel"}, {"name": "pypdf", "version": "7.0.0", "latest_version": "7.1.1", "latest_filetype": "wheel"}, {"name": "unsloth", "version": "2026.11.3", "latest_version": "2026.11.12", "latest_filetype": "wheel"}]"#;

    #[test]
    fn pip_versions_follow_pep_440() {
        for text in [format!("{PIP_OUTDATED}\n"), format!("{PIP_OUTDATED}\r\n")] {
            let found = parse_pip_outdated(&text).unwrap();
            assert_eq!(found.len(), 4);
            assert_eq!(found[3].id.native, "unsloth");
            assert_eq!(found[3].available.raw(), "2026.11.12");
            assert!(
                found
                    .iter()
                    .all(|c| c.installed.is_upgrade_to(&c.available))
            );
        }
        // `1.0.post1` is later than `1.0` and `2.0rc1` is a pre-release.
        let post = r#"[{"name":"a","version":"1.0","latest_version":"1.0.post1"},
                       {"name":"b","version":"1.0","latest_version":"2.0rc1"}]"#;
        let found = parse_pip_outdated(post).unwrap();
        assert_eq!(found.len(), 2);
        assert!(found[0].installed.is_upgrade_to(&found[0].available));
        assert!(!found[0].available.is_prerelease());
        assert!(found[1].available.is_prerelease());
        assert_eq!(found[0].available.raw(), "1.0.post1");
    }

    #[test]
    fn pip_failures_a_user_must_solve_get_a_clear_message() {
        // https://peps.python.org/pep-0668/, the message Debian ships.
        let pep668 = "error: externally-managed-environment\n\n\
                      x This environment is externally managed\n";
        assert!(explain_pip_failure(pep668).unwrap().contains("PEP 668"));
        let itself = "ERROR: To modify pip, please run the following command:\r\n\
                      C:\\Python314\\python.exe -m pip install --upgrade pip\r\n";
        assert!(
            explain_pip_failure(itself)
                .unwrap()
                .contains("python -m pip")
        );
        assert!(explain_pip_failure("ERROR: No matching distribution found").is_none());
        let out = proc::Output {
            code: 1,
            stdout: String::new(),
            stderr: pep668.into(),
        };
        assert!(matches!(
            pip_failed("pip install x".into(), out),
            Error::BackendUnavailable { .. }
        ));

        assert!(pip_self_update_refusal("pip", true).is_some());
        assert!(pip_self_update_refusal("Pip", true).is_some());
        assert!(pip_self_update_refusal("pip", false).is_none());
        assert!(pip_self_update_refusal("pipx", true).is_none());
    }

    #[test]
    fn pip_gets_the_time_it_needs() {
        assert!(SCAN_TIMEOUT.as_secs() >= 120);
        assert!(INSTALL_TIMEOUT.as_secs() >= 600);
    }

    #[test]
    fn parses_pip_list_json() {
        let out = r#"[{"name": "requests", "version": "2.32.3"},
                      {"name": "urllib3", "version": "2.2.2"}]"#;
        let installed = parse_pip_list(out).unwrap();
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].id.native, "requests");
        assert_eq!(installed[0].version, "2.32.3");
        assert_eq!(installed[1].name, "urllib3");
    }

    #[test]
    fn pip_list_reports_a_parse_error_rather_than_an_empty_inventory() {
        assert!(parse_pip_list("not json").is_err());
        assert!(parse_pip_outdated("not json").is_err());
        assert!(parse_pip_report("not json").is_err());
        assert!(parse_pip_inspect("not json").is_err());
    }

    /// Shape of `pip inspect`, captured from pip 26.3 and cut down.
    const PIP_INSPECT: &str = r#"{
      "version": "1", "pip_version": "26.3",
      "installed": [
        {"metadata": {"metadata_version": "2.4", "name": "accelerate", "version": "1.15.0",
          "requires_dist": ["numpy>=1.17", "packaging>=20.0", "huggingface_hub>=0.21.0",
                            "ruff==0.13.1; extra == \"quality\""]},
         "metadata_location": "C:\\x", "installer": "pip", "requested": true},
        {"metadata": {"name": "playwright", "version": "1.64.1",
          "requires_dist": ["pyee<15,>=13", "greenlet (<4.0.0,>=3.1.1)"]}},
        {"metadata": {"name": "old-thing", "version": "1.0",
          "requires_dist": ["numpy<2; python_version < \"3.8\"", "Huggingface.Hub~=1.30 ; sys_platform == \"win32\""]}},
        {"metadata": {"name": "numpy", "version": "2.1.0"}},
        {"metadata": {"name": "pyee", "version": "14.0.0"}},
        {"metadata": {"name": "greenlet", "version": "3.2.0"}},
        {"metadata": {"name": "huggingface_hub", "version": "1.33.0"}},
        {"metadata": {"name": "pypdf", "version": "7.0.0"}}
      ],
      "environment": {"os_name": "nt", "python_version": "3.14", "sys_platform": "win32"}
    }"#;

    fn plan(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn an_update_that_breaks_another_package_is_named_before_it_happens() {
        for text in [PIP_INSPECT.to_string(), crlf(PIP_INSPECT)] {
            let env = parse_pip_inspect(&text).unwrap();
            assert_eq!(env.installed.len(), 8);

            // Nothing needs pypdf.
            assert!(
                env.problems_with("pypdf", &plan(&[("pypdf", "7.1.1")]))
                    .is_empty()
            );
            // playwright pins pyee below 15.
            let broken = env.problems_with("pyee", &plan(&[("pyee", "16.0.1")]));
            assert_eq!(broken.len(), 1, "{broken:?}");
            assert!(broken[0].contains("playwright needs pyee<15,>=13"));
            // The pin is written with another spelling of the name, `~=1.30`
            // means below 2, and the marker matches this platform.
            let broken =
                env.problems_with("huggingface_hub", &plan(&[("huggingface-hub", "2.4.0")]));
            assert_eq!(broken.len(), 1, "{broken:?}");
            assert!(
                env.problems_with("huggingface_hub", &plan(&[("huggingface-hub", "1.40.0")]))
                    .is_empty()
            );
            // A marker that does not match, and an optional extra.
            assert!(
                env.problems_with("numpy", &plan(&[("numpy", "2.4.0")]))
                    .is_empty()
            );
            assert!(
                env.problems_with("ruff", &plan(&[("ruff", "1.0.0")]))
                    .is_empty()
            );
            // A dependency that would go backwards.
            let broken =
                env.problems_with("pypdf", &plan(&[("pypdf", "7.1.1"), ("greenlet", "3.1.5")]));
            assert_eq!(broken.len(), 1, "{broken:?}");
            assert!(broken[0].contains("go back"));
            // Replaced together: pip has solved that pair.
            assert!(
                env.problems_with(
                    "playwright",
                    &plan(&[("playwright", "1.65.2"), ("pyee", "16.0.1")])
                )
                .is_empty()
            );
        }
    }

    #[test]
    fn version_specifiers_follow_pep_440() {
        let accepts = |spec: &str, v: &str| {
            PipRequirement::parse(&format!("x{spec}"))
                .unwrap()
                .accepts(v)
        };
        assert!(accepts("", "1.0"));
        assert!(accepts("==1.4.*", "1.4.7"));
        assert!(!accepts("==1.4.*", "1.40.0"));
        assert!(accepts("!=1.4.*", "1.5"));
        assert!(accepts("~=1.4.2", "1.4.9"));
        assert!(!accepts("~=1.4.2", "1.5.0"));
        assert!(accepts("==2.1", "2.1.0+cu121"));
        assert!(accepts(">=1.0,<2,!=1.5", "1.6"));
        assert!(!accepts(">=1.0,<2,!=1.5", "1.5"));
        assert!(!accepts(">1.0", "1.0"));
        // What cannot be read is never "fine".
        assert!(!accepts(">=banana", "1.0"));
        assert!(!accepts("?1", "1.0"));

        assert!(PipRequirement::parse("x @ https://example.com/x.whl").is_none());
        let extras = PipRequirement::parse("Foo_Bar[a,b] (>=1) ; os_name == 'nt'").unwrap();
        assert_eq!(extras.name, "foo-bar");
        assert_eq!(extras.specifier, ">=1");
        assert_eq!(pip_normalize("A__b.-c"), "a-b-c");
    }

    #[test]
    fn the_installation_report_lists_every_change() {
        // https://pip.pypa.io/en/stable/reference/installation-report/,
        // shortened.
        let report = r#"{
          "version": "1", "pip_version": "22.2",
          "install": [
            {"metadata": {"name": "pydantic", "version": "1.9.1",
               "requires_dist": ["typing-extensions (>=3.7.4.3)", "python-dotenv (>=0.10.4) ; extra == 'dotenv'"]},
             "is_direct": false, "requested": true},
            {"metadata": {"name": "typing_extensions", "version": "4.3.0"}, "is_direct": false, "requested": false}
          ],
          "environment": {"python_version": "3.10"}
        }"#;
        for text in [report.to_string(), crlf(report)] {
            let changes = parse_pip_report(&text).unwrap();
            assert_eq!(
                changes,
                plan(&[("pydantic", "1.9.1"), ("typing_extensions", "4.3.0")])
            );
        }
    }

    #[test]
    fn pip_check_reports_only_what_is_newly_broken() {
        // Captured from pip 26.3 on Windows.
        let before = "playwright 1.64.1 has requirement pyee<15,>=13, but you have pyee 16.0.1.\n";
        let after = format!("{before}foo 1.0 requires bar, which is not installed.\n");
        for (b, a) in [
            (before.to_string(), after.clone()),
            (crlf(before), crlf(&after)),
        ] {
            let (b, a) = (parse_pip_check(&b), parse_pip_check(&a));
            assert_eq!(b.len(), 1);
            assert_eq!(a.len(), 2);
            assert_eq!(
                newly_broken(&b, &a),
                vec!["foo 1.0 requires bar, which is not installed.".to_string()]
            );
            assert!(newly_broken(&a, &b).is_empty());
        }
        assert!(parse_pip_check("No broken requirements found.\r\n").is_empty());
    }

    // cargo.

    #[test]
    fn parses_cargo_install_list() {
        let out = "\
ripgrep v14.1.0:
    rg
cargo-edit v0.12.2 (/home/u/src/cargo-edit):
    cargo-add
    cargo-rm
";
        let installed = parse_cargo_install_list(out);
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].id.native, "ripgrep");
        assert_eq!(installed[0].version, "14.1.0");
        // The path suffix is not part of the version, and the indented binary
        // names are not packages.
        assert_eq!(installed[1].id.native, "cargo-edit");
        assert_eq!(installed[1].version, "0.12.2");
    }

    #[test]
    fn cargo_install_list_empty_output_yields_empty_vec() {
        assert!(parse_cargo_install_list("").is_empty());
    }

    #[test]
    fn only_crates_io_installs_are_compared_with_crates_io() {
        // Captured on Windows, plus one path and one git install.
        let out = "\
cargo-audit v0.22.2:
    cargo-audit.exe
cargo-cyclonedx v0.5.9:
    cargo-cyclonedx.exe
mine v0.1.0 (C:\\src\\mine):
    mine.exe
theirs v0.2.0 (https://github.com/x/theirs#0a1b2c3d):
    theirs.exe
";
        for text in [out.to_string(), crlf(out)] {
            let found = parse_crates_io_installs(&text);
            assert_eq!(
                found,
                plan(&[("cargo-audit", "0.22.2"), ("cargo-cyclonedx", "0.5.9")])
            );
        }
    }

    #[test]
    fn the_index_path_follows_the_registry_layout() {
        // https://doc.rust-lang.org/cargo/reference/registry-index.html#index-files
        let url = |n| crates_io_index_url(n).unwrap();
        assert_eq!(url("a"), "https://index.crates.io/1/a");
        assert_eq!(url("ab"), "https://index.crates.io/2/ab");
        assert_eq!(url("abc"), "https://index.crates.io/3/a/abc");
        assert_eq!(url("Cargo"), "https://index.crates.io/ca/rg/cargo");
        assert!(crates_io_index_url("").is_none());
        assert!(crates_io_index_url("-x").is_none());
        assert!(crates_io_index_url("../etc").is_none());
        assert!(crates_io_index_url("a/b").is_none());
    }

    #[test]
    fn yanked_and_pre_releases_are_never_the_newest() {
        // Line shape from the registry index documentation (shortened).
        let body = r#"{"name":"foo","vers":"0.9.0","deps":[],"cksum":"d867","features":{},"yanked":false}
{"name":"foo","vers":"0.10.0","deps":[],"cksum":"d867","features":{},"yanked":false}
{"name":"foo","vers":"0.10.1","deps":[],"cksum":"d867","features":{},"yanked":true}
{"name":"foo","vers":"1.0.0-rc.1","deps":[],"cksum":"d867","features":{},"yanked":false}
{"name":"foo","vers":"1.0.0-0","deps":[],"cksum":"d867","features":{},"yanked":false}
"#;
        assert_eq!(newest_published(body).as_deref(), Some("0.10.0"));
        assert_eq!(newest_published(&crlf(body)).as_deref(), Some("0.10.0"));
        assert_eq!(newest_published(""), None);
        assert_eq!(newest_published("<html>"), None);
    }

    #[test]
    fn crates_with_their_own_features_are_left_alone() {
        // Captured from `.crates2.json` and shortened; the second and third
        // entry are made up from its shape.
        let json = r#"{"installs":{
          "cargo-audit 0.22.2 (registry+https://github.com/rust-lang/crates.io-index)":{"version_req":null,"bins":["cargo-audit.exe"],"features":[],"all_features":false,"no_default_features":false,"profile":"release","target":"x86_64-pc-windows-msvc","rustc":"rustc 1.99.1"},
          "ripgrep 14.1.0 (registry+https://github.com/rust-lang/crates.io-index)":{"version_req":null,"bins":["rg.exe"],"features":["pcre2"],"all_features":false,"no_default_features":false,"profile":"release"},
          "wasm-pack 0.15.0 (registry+https://github.com/rust-lang/crates.io-index)":{"version_req":"0.15.0","bins":["wasm-pack.exe"],"features":[],"all_features":false,"no_default_features":true,"profile":"release"}}}"#;
        let custom = parse_custom_builds(json).unwrap();
        assert_eq!(custom.len(), 2);
        assert!(custom.contains("ripgrep"));
        assert!(custom.contains("wasm-pack"));
        assert!(parse_custom_builds("broken").is_err());
        assert!(parse_custom_builds("{}").unwrap().is_empty());
    }

    // npm.

    #[test]
    fn parses_npm_global_list_json() {
        let out = r#"{
            "name": "lib",
            "dependencies": {
                "npm": {"version": "10.5.0"},
                "typescript": {"version": "5.5.4"}
            }
        }"#;
        let mut installed = parse_npm_global_list(out).unwrap();
        installed.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].id.native, "npm");
        assert_eq!(installed[0].version, "10.5.0");
        assert_eq!(installed[1].version, "5.5.4");
    }

    #[test]
    fn scoped_npm_packages_are_found_by_their_full_name() {
        // Captured from npm on Windows; only the name was replaced.
        let out = "{\n  \"name\": \"npm\",\n  \"dependencies\": {\n    \"@angular/cli\": {\n      \"version\": \"2.1.319\",\n      \"overridden\": false\n    }\n  }\n}\n";
        for text in [out.to_string(), crlf(out)] {
            let installed = parse_npm_global_list(&text).unwrap();
            assert_eq!(installed.len(), 1);
            assert_eq!(installed[0].id.native, "@angular/cli");
            assert_eq!(installed[0].version, "2.1.319");
        }
    }

    #[test]
    fn npm_outdated_is_read_and_its_failures_are_not_swallowed() {
        // First entry captured from npm on Windows; name and path were replaced.
        let out = r#"{
  "@angular/cli": {
    "current": "2.1.319",
    "wanted": "2.1.332",
    "latest": "2.1.332",
    "dependent": "global",
    "location": "C:\\npm\\node_modules\\@angular\\cli"
  },
  "mylib": {"current": "1.0.0", "wanted": "linked", "latest": "9.9.9"},
  "gone": {"wanted": "1.0.0", "latest": "1.0.0"}
}
"#;
        for text in [out.to_string(), crlf(out)] {
            let found = parse_npm_outdated(&text).unwrap();
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].id.native, "@angular/cli");
            assert_eq!(found[0].installed.raw(), "2.1.319");
            assert_eq!(found[0].available.raw(), "2.1.332");
        }
        assert!(parse_npm_outdated("").unwrap().is_empty());
        assert!(parse_npm_outdated("{}").unwrap().is_empty());
        assert!(parse_npm_outdated("npm ERR! network").is_err());
        let err = r#"{"error": {"code": "ENOTFOUND", "summary": "request to https://registry.npmjs.org/npm failed", "detail": ""}}"#;
        assert!(matches!(
            parse_npm_outdated(err).unwrap_err(),
            Error::CommandFailed { .. }
        ));
        // A package that is really called "error".
        let named = r#"{"error": {"current": "1.0.0", "wanted": "1.1.0", "latest": "1.1.0"}}"#;
        assert_eq!(parse_npm_outdated(named).unwrap().len(), 1);
    }

    #[test]
    fn npm_global_list_skips_entries_without_a_version() {
        let out = r#"{"dependencies": {"broken": {"missing": true}, "ok": {"version": "1.0.0"}}}"#;
        let installed = parse_npm_global_list(out).unwrap();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].id.native, "ok");
    }

    #[test]
    fn npm_global_list_reports_failures() {
        assert!(parse_npm_global_list("npm ERR! something").is_err());
        assert!(parse_npm_global_list(r#"{"error": {"code": "EACCES"}}"#).is_err());
        assert!(parse_npm_global_list("{}").unwrap().is_empty());
    }

    // go.

    /// Format from https://go.dev/ref/mod#go-version-m; the goimports block is
    /// the documentation's own example.
    const GO_VERSION_M: &str = "\
/home/jrgopher/go/bin/goimports: go1.14.3
\tpath\tgolang.org/x/tools/cmd/goimports
\tmod\tgolang.org/x/tools\tv0.0.0-20200518203908-8018eb2c26ba\th1:0Lcy64USfQQL6GAJma8BdHCgeofcchQj+Z7j0SXYAzU=
\tdep\tgolang.org/x/mod\tv0.2.0\th1:KU7oHjnv3XNWfa5COkzUifxZmxp1TyI7ImMXqFxLwvQ=
\tdep\tgolang.org/x/xerrors\tv0.0.0-20191204190536-9bdfabe68543\th1:E7g+9GITq07hpfrRu66IVDexMakfv52eLZ2CXBWiKr4=
/home/jrgopher/go/bin/gopls: go1.22.0
\tpath\tgolang.org/x/tools/gopls
\tmod\tgolang.org/x/tools/gopls\tv0.15.1\th1:abc=
\tbuild\t-buildmode=exe
/home/jrgopher/go/bin/local: go1.22.0
\tpath\texample.com/me/local
\tmod\texample.com/me/local\t(devel)\t
/home/jrgopher/go/bin/forked: go1.22.0
\tpath\texample.com/forked
\tmod\texample.com/forked\tv1.0.0
\t=>\t./fork\t(devel)
\tdep\texample.com/other\tv1.2.0\th1:abc=
\t=>\texample.com/other\tv1.1.0\th1:abc=
";

    #[test]
    fn only_published_go_tools_are_found() {
        for text in [GO_VERSION_M.to_string(), crlf(GO_VERSION_M)] {
            let tools = parse_go_version_m(&text);
            assert_eq!(tools.len(), 2, "{tools:?}");
            assert_eq!(tools[0].path, "golang.org/x/tools/cmd/goimports");
            assert_eq!(tools[0].module, "golang.org/x/tools");
            assert_eq!(tools[1].path, "golang.org/x/tools/gopls");
            assert_eq!(tools[1].version, "v0.15.1");
        }
        assert!(parse_go_version_m("").is_empty());
    }

    #[test]
    fn the_go_tool_directory_is_gobin_or_the_first_gopath() {
        assert_eq!(
            go_bin_dir(r#"{"GOBIN":"/opt/gobin","GOPATH":"/home/u/go"}"#).unwrap(),
            Some("/opt/gobin".into())
        );
        let from_path = go_bin_dir(r#"{"GOBIN":"","GOPATH":"gopath"}"#)
            .unwrap()
            .unwrap();
        assert_eq!(
            std::path::Path::new(&from_path),
            std::path::Path::new("gopath").join("bin")
        );
        assert_eq!(go_bin_dir("{}").unwrap(), None);
        assert!(go_bin_dir("go: unknown").is_err());
    }

    #[test]
    fn the_newest_go_module_version_is_kept_verbatim() {
        let json = "{\n\t\"Path\": \"golang.org/x/tools/gopls\",\n\t\"Version\": \"v0.16.0\",\n\t\"Time\": \"2024-06-01T00:00:00Z\"\n}\n";
        assert_eq!(parse_go_module_version(json).as_deref(), Some("v0.16.0"));
        assert_eq!(parse_go_module_version("{}"), None);
        assert_eq!(parse_go_module_version(r#"{"Version":"-x"}"#), None);
    }
}
