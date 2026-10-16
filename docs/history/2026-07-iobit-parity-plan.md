# Odysync — Plan: full IObit-suite parity, done safely

Written 2026-07-27, against `main` @ `3adeeed`. Every claim below was checked
against the source, not against `README.md`.

The goal in one sentence: **Odysync should replace IObit Software Updater,
Driver Booster and Advanced SystemCare with one tool that never breaks a
working machine — and it must also install Windows' own updates.**

---

## 0. Where the code actually is today

Verified locally on this clone:

| Check | Result |
|---|---|
| `npm install` (apps/gui) | 116 packages, **0 vulnerabilities** |
| `npx tsc -b` | clean, exit 0 |
| `npx oxlint` | exit 0, 4 `only-export-components` warnings (cosmetic) |
| `cargo build --workspace --all-targets` | clean, 4m24s, zero warnings |
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets` with `-D warnings` | clean |
| `cargo test --workspace --lib` | **440 passed, 0 failed** (README says 439) |
| CI | `rust.yml` runs fmt + clippy + tests + release build + GUI build on 3 OSes, plus `cargo-audit`, `cargo-deny`, CycloneDX SBOM |

The repo is **much further along than `README.md` and `ROADMAP.md` admit**.
README says "6 backends"; `all_backends()` registers **44 distinct backends**
(36 of them compiled in on Windows, 42 on Linux, 38 on macOS):

- Windows package managers: winget, Microsoft Store, Chocolatey, Scoop
- GPU: NVIDIA, AMD, Intel, Qualcomm
- OEM tools: Dell Command Update, HP Image Assistant, Lenovo System Update,
  MSI Center, ASUS Armoury, Gigabyte Control Center, Acer Care Center,
  Razer Synapse
- Firmware: Dell, HP, Lenovo; `fwupd` (Linux), Mac firmware
- Windows drivers via the Windows Update Agent COM API
- Language ecosystems: pip, cargo, npm, go, dotnet tool, VS Code extensions,
  JetBrains plugins, PowerShell modules, Windows optional features
- Unix: Homebrew, apt, dnf, pacman, zypper, snap, flatpak, nix, AppImage

The GUI has 17 pages (Dashboard, Updates, Drivers, Hardware, Maintenance,
Security, Startup, Schedule, Backup, Offline, Packages, Profiles, History,
Logs, Settings, About). The security module is ~6 000 lines across Defender,
persistence, integrity, network, posture and remediation.

**First action item: `README.md` and `ROADMAP.md` are stale and undersell the
project.** Fixing them is cheap and is part of Phase A.

Toolchain used: rustc 1.96.1, cargo 1.96.1, Node 24.14.1, npm 11.11.0.

**The tree is healthy.** Nothing in Phase A below is repair work; it is
verification and documentation catch-up.

---

## 1. Gap analysis against the three products the user named

| IObit product | Odysync today | Gap |
|---|---|---|
| **Software Updater** | 44 backends across 3 OSes, safety policy, holds/pins, offline cache, profiles | None worth naming. Odysync is already broader. |
| **Driver Booster** (hardware) | WUA driver search, 4 GPU vendors, 8 OEM tools, 3 firmware vendors, restore point before apply | **No driver backup, no driver rollback.** This is Driver Booster's single most valuable feature and the one that makes driver updating safe. |
| **Advanced SystemCare** | 4 maintenance actions (temp, recycle bin, DISM/SFC, startup viewer) + a real security audit IObit does not have | **Deep clean missing**: browser/privacy traces, WinSxS component cleanup, Delivery Optimization cache, `Windows.old`, large/duplicate files, disk optimize/TRIM, per-app startup impact. |
| **Windows Update itself** | Only `Type='Driver'` is searched (`windows_drivers.rs:137`) | **Windows quality, security and definition updates are not installed at all.** Largest functional gap. |

---

## 2. Defects found in the audit

These are things the code claims but does not do. They matter more than new
features, because they are trust claims.

1. **`odysync-verify` is never called.** The crate exists (188 + 175 lines,
   SHA-256 + Authenticode/codesign), is declared as a dependency in
   `crates/odysync-backends/Cargo.toml:16`, and has **zero call sites** in the
   whole workspace. `README.md` advertises "installer digest verification and
   signature checking" as a feature. Right now that is a dependency edge, not
   a behaviour. `offline.rs:447` does its own SHA-256 check, unrelated.

2. **No Windows OS updates.** `windows_drivers.rs` hard-codes
   `Type='Driver'` in both the scan and the install path. There is no
   `BackendKind::WindowsUpdate`.

3. **No driver backup or rollback.** `ROADMAP.md` is honest that rollback is
   "prevention, not undo" — but for drivers specifically, Windows gives us
   `pnputil /export-driver` and DriverStore rollback for free. Not using it is
   a choice we should reverse.

4. **Releases are unsigned.** No Authenticode, no notarisation, no self-update.
   Every install of Odysync shows a SmartScreen warning, which trains users to
   click through exactly the warning that protects them. `ROADMAP.md` §5 names
   this asymmetry and it is still open.

5. **No power/idle awareness** (`ROADMAP.md` §4) — a background scanner that
   runs on battery is a laptop problem.

6. **The driver install path never downloads and never accepts a EULA.**
   `install_driver_com` goes straight from `SetUpdates` to `Install`, with no
   `IUpdateDownloader` step and no `AcceptEula()`. `IUpdateInstaller::Install`
   requires `IsDownloaded == true` and fails with `WU_E_NOT_DOWNLOADED`
   (`0x80240034`) otherwise, so a driver install only succeeds when Windows
   happens to have cached the update already. Phase B's shared WUA wrapper
   fixes this for both backends at once.

7. **`RunReport::reboot_required` is never set to `true`.** The field is
   initialised `false` in `report.rs:36`, read in the CLI renderer and in three
   GUI components (`Dashboard.tsx:49`, `Drivers.tsx:184`, `Updates.tsx:238`) —
   and assigned nowhere. `install_driver_com` discards
   `IInstallationResult::RebootRequired()`. Three "a reboot is required"
   banners exist that can never appear.

8. **The driver backend has never been able to install a driver.** This is the
   worst of them. `scan_drivers_com` sets
   `available = Version::parse("{update_guid}.{revision}")`. `Version::parse`
   splits at the first `-` that follows a digit, so the whole version reduces
   to the GUID's first block — `22cb1a63` — which has no numeric segment and
   therefore parses to `Version::Unknown`. Verified by running the real parser
   over real-shaped IDs: 3 of 4 sample GUIDs came back `is_known=false`, the
   fourth only because its first block happened to be all digits.

   The consequences compound. The policy engine blocks every driver candidate
   with `UnknownAvailableVersion`, and even with `require_known_versions`
   turned off, `WindowsDriverBackend::apply` refuses outright on
   `!candidate.available.is_known()`. So the entire Driver Booster half of the
   product — the Hardware page, the driver scan, the apply button — can list
   updates and can never install one.

   Fix: Windows updates have no meaningful installed-version to compare
   against. Model them honestly as `0` → `revision`, which is a real ordered
   pair, and let the KB number and title carry the identity.

9. **~~The npm and VS Code backends never worked on Windows.~~ Fixed.**
   `proc::build` called `Command::new("npm")`, and `CreateProcess` only ever
   appends `.exe` when searching `PATH`. npm ships as `npm.cmd` and VS Code's
   CLI as `code.cmd`, so both backends reported `is_available() == false` on
   every Windows machine — silently, because an unavailable backend is skipped
   rather than reported. `proc::resolve_program` now walks `PATH` against
   `PATHEXT` before spawning, without routing through a shell. Verified on this
   host: `odysync backends` went from 9 detected to 11, gaining `npm` and
   `vscode-extension`.

---

## 3. The plan

Six phases. Each is independently shippable and does not require the next one.

### Phase A — Trust the baseline (½ day)

Nothing new is built until the existing thing is proven.

- [x] `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace --lib` — all green locally, matching CI.
- [x] `cargo build --release -p odysync-cli` and a real smoke test.
      2.31 MB binary; `odysync backends` detects 11 on this host;
      `odysync scan` returned **50 real updates** (37 winget, plus pip).
- [x] Fixed the `PATHEXT` defect above; 3 regression tests added
      (`odysync-core` went from 75 to 78 tests, 443 in the workspace).
- [ ] `npx tauri build`, install the NSIS output, click every one of the 17
      pages, note anything that errors or hangs.
- [ ] Rewrite the stale parts of `README.md` (backend count, feature list) and
      `ROADMAP.md` (phases marked open that are done).
- [ ] *(optional, low value)* The 4 oxlint `only-export-components` warnings
      are a Fast-Refresh nicety, not a defect — `oxlint` exits 0. Worth doing
      only when `ui.tsx`/`store.tsx` are being touched anyway.

**Exit:** we know exactly what works, from evidence rather than documentation.

### Phase B — Windows Update — *the headline gap* — **largely done**

- [x] COM plumbing extracted from `windows_drivers.rs` into
      `windows_update/session.rs`, shared by all four WUA backends. Defects
      6, 7 and 8 are fixed there, for drivers and software at once: an
      `IUpdateDownloader` pass before `Install`, `AcceptEula()` before the
      download, `RebootRequired` no longer discarded, and an orderable version
      pair instead of a GUID that parsed to `Unknown`.
- [x] Search `IsInstalled=0 and Type='Software' and IsHidden=0`, classified by
      **category GUID, never by name** — category names are localised.
      Verified against a live German machine: 14 pending updates, and their
      GUIDs matched the constants.
- [x] Three backends rather than one switch: `windows-update` (security +
      quality), `windows-defender-update` (definitions),
      `windows-feature-update` (version upgrades). The split falls out of the
      existing architecture — the CLI, the GUI, holds, pins and profiles pick
      it up with no front-end change.
- [x] **Feature upgrades are opt-in**, via `BackendKind::enabled_by_default`
      rather than a default `disabled-backends` entry. A default list entry
      would never reach a user whose config file was written by an older
      build; a method on the kind applies to everyone.
- [x] `apply` re-checks the class against a fresh search rather than trusting
      the candidate handed back across the GUI's process boundary, so the
      security backend can never be talked into installing an upgrade.
- [x] Reboot state read from the OS (`platform::reboot_pending`) rather than
      from one backend's install result, so it covers every backend including
      a winget installer that queued a pending file rename.
- [x] `explain_hresult` turns the codes that actually occur into sentences —
      "another installation is already in progress", "this needs to run
      elevated" — instead of a bare `0x80240016`.
- [x] Verified end to end, unelevated: 3 driver updates and 13 Windows updates
      listed, each with `requires administrator privileges` as its inline skip
      reason. Before this work, none of them appeared at all.

- [x] **Hold by KB number.** `PackageId` gained an optional alias that the
      policy engine matches alongside the native id, so `odysync hold
      KB5101650` works while the native id stays `<guid>.<revision>` for the
      Update Agent. The alias is excluded from equality and hashing — a label
      must not change a package's identity when `PackageId` is a map key.
      Verified against a real pending update: the skip reason changed from
      *requires administrator privileges* to *held by policy*, which also
      confirms holds outrank the elevation rule as the engine intends.
- [x] **`odysync pause <backend> --days N` / `resume <backend>`.** Modelled as
      a deadline rather than an off switch, and generic over backends rather
      than special-cased to Windows. Verified: pausing `windows-update` hid it
      while leaving `windows-defender-update` available.

- [x] **GUI: a Windows Update card** above the generic list, breaking the
      pending updates down by class with the blocking reason per row. It exists
      because the generic list flattens a real distinction: on an unelevated
      run every Windows update lands in `skipped`, so 13 pending security
      updates sat inside a list of 49 skipped entries and read as "nothing to
      do". Rendered in isolation against three fixtures — unelevated, elevated
      with an opted-in feature upgrade, and non-Windows — which caught two
      defects the type-checker could not: a `title` attribute that replaced the
      visible "10 blocked" as the accessible name, and a feature-upgrade note
      that told the user how to enable something already enabled.

Remaining:

- [ ] Install has not been exercised: it needs elevation, and an elevated run
      was out of reach here. The scan half is verified against the real API;
      the install half is verified only by construction and unit tests.
- [ ] No test runner for the frontend. `tsc` and `oxlint` catch types and lint,
      but nothing asserts a component's rendered output — the two defects above
      were found by looking, which does not scale. Vitest plus Testing Library
      would be a small addition with a real payoff.

**Risk:** WUA is a COM API with real failure modes (WSUS-managed machines,
policy-blocked updates). `explain_hresult` covers the common ones; the rest
still surface their raw code, which is better than silence.

### Phase C — Make the safety claims true — **done**

The plan for this phase was wrong, and finding out why was the useful part.

It assumed backends hand the runner a downloaded installer path that the runner
could then verify. **No backend does.** Odysync asks winget, apt, Homebrew or
the Windows Update Agent to perform the update; those tools download, verify
and execute the payload inside their own process, and no file ever exists at a
path we control. Wiring `odysync-verify` into the runner would have produced a
hook that never fires.

There is exactly one place Odysync downloads an executable itself — the offline
cache — and that is where the crate belongs.

- [x] **The offline cache is now the real call site.** `download_and_cache`
      writes to a `.partial` staging name, runs `odysync_verify::verify_signature`
      on it, and only renames it into place if it passes. A file whose
      signature is *invalid* is deleted and the download fails: unsigned is a
      state the world is genuinely in, but a signature that does not validate
      means the bytes changed after signing or the certificate was revoked.
- [x] **HTTPS only.** `download_and_cache` took an arbitrary URL from the
      front-end and would happily fetch an installer over plaintext `http://`,
      where it can be swapped in transit — and `expected_sha256` is optional,
      so frequently nothing would have caught the swap. `file://` and other
      schemes are refused too.
- [x] A malformed `expected_sha256` is now rejected as a caller bug rather than
      silently degrading to "does not match".
- [x] The signature verdict is **recorded in the manifest** (`CachedSignature`),
      with `#[serde(default)]` so older manifests still load and read as
      `Unknown` — honest, since nothing looked at them.
- [x] **`odysync_core::verification_of`** answers "who checked this?" for all
      47 backends, and `odysync backends` prints it: *verified by winget*,
      *verified by the Windows Update Agent*, *verified by apt*. AppImage is
      reported as **not verified**, with the reason. A test asserts no backend's
      text can be read as "Odysync verified it".
- [x] `npm audit --audit-level=moderate` added to the `gui` CI job.
- [x] Dependabot configured for cargo, npm and GitHub Actions — grouped weekly
      so the PRs are reviewable rather than ignored.

Deliberately **not** done: a publisher allow-list. With no installer to inspect
on the apply path, the only place it could apply is the offline cache, and an
allow-list of one is a configuration burden rather than a control. Revisit if a
backend ever downloads its own payload.

**Exit:** every verification statement in the UI names who performed the check,
and the one file Odysync fetches itself is actually checked.

### Phase D — Driver safety: backup and rollback — **done**

- [x] `driver_backup` module: enumerate, export per package into
      `%LOCALAPPDATA%\SenseiIssei\Odysync\data\driver-backup\<id>\`, list,
      restore, prune.
- [x] `odysync drivers backup | list | rollback --to <id> | prune --keep <n>`.
- [x] Retention via `prune`, wired to `driver-backup-keep` (default 2).
- [x] Verified against the real driver store: 215 packages enumerated (matching
      an independent count), one exported to 10 files / 22.5 MB, manifest
      written, listed and pruned cleanly.

**Nothing here parses a label.** `pnputil` output is fully localised — on this
machine `Published Name` reads `Veröffentlichter Name`, with the umlaut
mangled by the console code page on the way out. The parser reads *values*: an
`oem<n>.inf` token is a published name in every locale, and the other `.inf`
token in the same record is the original name. Export and restore go through
exit codes alone.

**The bug that unit tests could not have caught.** The first parser split
blocks on `"\n\n"`. `pnputil` is a Windows console program, so its blank line
is `\r\n\r\n` and that split never matches — the whole output collapsed into
one block and the parser returned **1 package on a machine with 215**. The
tests passed, because they were written with `\n` and so only ever tested their
own assumption. Every parser test now runs against both line endings, and one
asserts a record *count* specifically, because the failure mode returns exactly
one item no matter how many are present.

**Automatic pre-apply backup is off by default, from a measurement.**
`DriverStore\FileRepository` is **4.7 GB across 5 229 files** here, and the
single package exported during verification was 22.5 MB — so a full export is
around 5 GB, twice over at the default retention. Spending ten-plus gigabytes
of the user's disk automatically, on the machine we are meant to be looking
after, is not a defensible default, and Windows already keeps a single-step
rollback in Device Manager. So `odysync apply` **says which of the two the user
is getting**: it names the most recent backup, or states plainly that there is
none and how to take one. `driver-backup-before-apply` turns the automatic
export on for those who want it.

**Restore is honest about its limits.** `pnputil /add-driver /install` re-adds
the saved package and installs it where it applies — it is not a forced
downgrade. Windows ranks driver packages, and a newer one still in the store
can keep winning; removing that newer package is destructive and is left to the
user in Device Manager rather than done automatically. The CLI prints this
every time rather than only in the docs.

Remaining:

- [ ] Restore has not been exercised: writing to the driver store needs
      elevation, which was out of reach here. Export, list and prune are
      verified against the real system; restore is covered by unit tests only.
- [ ] GUI button on the Hardware page.

### Phase E — Advanced SystemCare parity, minus the snake oil — **done**

Every cleaner follows one contract: **scan first, show sizes, delete only what
was ticked, never touch anything outside a declared root.**

- [x] `odysync clean` measures ten categories and deletes nothing. `--apply`
      is required to remove anything, `--only` to touch a category that is not
      safe by default, and anything irreversible is named individually before
      the prompt rather than folded into a count.
- [x] Categories: temp files, thumbnail and icon caches, Recycle Bin, error
      reports, crash dumps, Windows Update downloads, Delivery Optimization,
      browser caches, superseded components, `Windows.old`.
- [x] **Three independent path checks** before anything is removed, so a
      mistake in one is caught by the next: roots are filtered through
      `is_safe_cleanup_root` (an unset `%LOCALAPPDATA%` collapsing a root to
      `\Temp` is dropped, not acted on); the walk never follows a directory
      symlink, so a junction inside a cache cannot lead it elsewhere; and every
      individual path is re-checked with `is_within` immediately before
      deletion.
- [x] Browser caches for Chrome, Edge and Firefox, per profile, **refused while
      the browser is running** — and only `Cache`, `Code Cache`, `GPUCache`,
      `cache2`. Never `Cookies`, `Login Data` or `Network`. A test asserts no
      declared directory is a credential store.
- [x] `odysync optimize-disk`: `Optimize-Volume`, **TRIM on SSD, defrag on
      HDD**, chosen from the media type with no user override — defragmenting
      an SSD writes the whole drive for no benefit.
- [x] `Windows.old` is **measured and never deleted**. It is owned by
      TrustedInstaller, and a half-working reimplementation of Disk Cleanup
      that fails partway leaves the tree in a state neither tool understands.
      The size is the useful part; the user is pointed at Settings → Storage.
- [x] `DISM /StartComponentCleanup` **without `/ResetBase`** — that flag
      additionally makes every installed update permanently un-uninstallable,
      which is a much larger promise than "reclaim superseded components".

**Verified against this machine**, and the measurements were checked against
independent counts rather than trusted:

| Category | Odysync | Independent check |
|---|---|---|
| Windows Update downloads | 9621.4 MB | 9621.4 MB — exact |
| Temp files | 1115.9 MB | 1115.9 MB — exact |
| Recycle Bin | 879.9 MB | 881.7 MB across both drives |
| Browser caches | 1226.8 MB | Chrome, Edge and Firefox all found |

The Recycle Bin case is worth recording: the obvious PowerShell check
(`Shell.Application.NameSpace(0xA).Items().Size`) reported **1.1 MB**, because
`.Size` does not recurse into deleted folders and it missed `D:` entirely.
`SHQueryRecycleBinW` is authoritative, which is also why the code does not walk
`$Recycle.Bin` itself.

**Deliberately not built, for the same reason as the registry cleaner:**

- **A startup "impact" score.** `Explorer\StartupApproved` gives the
  enabled/disabled state, which Odysync already lists — but Task Manager's
  impact rating comes from undocumented telemetry that cannot be reproduced
  faithfully. A number that looks like Task Manager's and disagrees with it is
  worse than no number. Offering *delay* as well as disable is still a good
  idea and remains open; it does not need a fake score.
- **Large-file and duplicate reporting.** Genuinely useful and genuinely
  report-only, but a full-disk hash walk is expensive and was not something
  that could be verified properly here. Left open rather than shipped
  unverified.

### Phase F — Ship it properly — **code done, procurement open**

- [x] **Signing wired into `release.yml`**, for both the CLI binaries and the
      NSIS installer, with an RFC 3161 timestamp. Gated on a `SIGNING_ENABLED`
      repository variable so the workflow keeps working today and starts
      signing the moment the credentials exist — rather than being a separate
      change somebody has to remember at the same moment the validation clears.
- [x] `docs/SIGNING.md`: why it matters more than the friction suggests, the
      certificate comparison, the exact variables and secrets to set, how to
      verify a signed artifact, and what order to do things in afterwards.
- [x] **SHA-256 published for the installer**, not only the CLI archives.
- [x] **winget manifests generated per release** into a CI artifact, ready to
      submit to `microsoft/winget-pkgs`.
- [x] **`odysync self-check`** — reports a newer release, compares with the
      policy engine's version algebra rather than string order, surfaces the
      published checksum, and **refuses to install**.
- [x] **The daemon no longer scans on battery**, and logs its resident memory
      at startup so the 15 MB target is measured rather than assumed.
- [x] Version bumped to 2.2.0 across the workspace, the Tauri config and the
      GUI package.

**Self-update is deliberately check-only, and the ordering is not a
preference.** `ARCHITECTURE.md` criticises v1 for fetching a PowerShell module
at runtime as Administrator and trusting whatever answered. Downloading an
unsigned executable and running it elevated is the same hole with our own name
on it. `UpdateCheck::is_installable` is the single place that decision lives,
the verification it will need already exists in `odysync-verify`, and the test
`odysync_refuses_to_install_its_own_updates_while_releases_are_unsigned` is
what will fail — deliberately — on the day signing lands.

Remaining, and not something code can finish:

- [ ] **Buy the certificate.** Azure Trusted Signing, ~10 USD/month. Identity
      validation takes days and requires the identity to have existed for three
      years; if that fails, an OV certificate is the fallback. This is the last
      thing between the project and a release a stranger can install without
      being trained to click through a SmartScreen warning.
- [ ] Tag `v2.2.0`. Not done here: the work is on a branch, and publishing an
      unsigned release in the same change that adds the signing pipeline would
      be an odd thing to do. Merge, then sign, then tag.
- [ ] macOS signing and notarisation; Linux `.deb`/`.rpm`/AppImage.

---

## 4. What I recommend *not* building

Stated plainly, because these are the parts of Advanced SystemCare that sell
copies and damage machines.

1. **A registry cleaner that writes.** Measured benefit on a modern Windows
   install is indistinguishable from zero; the failure mode is an unbootable
   system. **Decided: report-only.** Orphaned entries are found and shown;
   Odysync never edits the registry to "clean" it.
2. **"RAM boost" / memory optimisers.** Freeing working sets forces the pages
   straight back off disk. It makes the machine slower and the graph prettier.
3. **Network "tuning"** via TCP registry tweaks. Windows autotunes; these
   settings are cargo-cult. A diagnostic report (DNS latency, packet loss,
   driver version) is genuinely useful — the tweaks are not.
4. **Automatic feature upgrades.** A cumulative update is routine; a Windows
   version upgrade is not something a background daemon should start.

---

## 5. Order and effort

| Phase | Effort | Why this order |
|---|---|---|
| A — verify baseline | ½ day | Never build on unverified ground |
| F₀ — start signing procurement | 1 h | Weeks of external lead time; start it on day one |
| B — Windows Update | 2–3 d | The named gap, and the highest user value |
| C — make verify real | 1–2 d | Trust claims outrank new features |
| D — driver backup/rollback | 2 d | Makes Phase B's driver half safe |
| E — deep clean | 3–4 d | Largest surface, lowest risk if the contract in §E holds |
| F — release | 1 d + lead time | Signed, self-updating, winget-installable |

Total: roughly two working weeks of focused effort, plus signing lead time.
