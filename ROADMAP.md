# Odysync v2 — Roadmap

Status as of 2026-07-20. Branch: `dev/rust-rewrite`.

Each phase is independently shippable. Nothing below requires a rewrite of
anything above it — the crate boundaries from Phase 1 are what make that true.

---

## Phase 1 — Core engine + CLI ✅ **DONE**

Commit `dc54a2c`. 100 unit tests, clippy-clean, warning-free, 1 MB binary,
CI on Windows/macOS/Linux.

Delivered: the safety policy, version algebra, planner, runner, verification
crate, five backends (winget, msstore, homebrew, apt, flatpak), and the
`odysync` CLI (`scan`, `apply`, `backends`, `hold`, `unhold`, `config`).

The four defects that corrupted installs are fixed and covered by regression
tests. See `ARCHITECTURE.md` for where each guarantee is enforced.

---

## Phase 2 — Windows feature parity

**Goal:** everything v1 could do, v2 can do. This is what unblocks deleting
`legacy/`.

### 2.1 Driver updates
- [x] Replace PSWindowsUpdate with the **Windows Update Agent COM API**
      (`IUpdateSession` / `IUpdateSearcher`) via the `windows` crate.
- [x] Driver-only search filter (`Type='Driver'`), surfaced as `BackendKind::WindowsDrivers`.
- [x] Report reboot-required through `RunReport::reboot_required`.

> **Why change:** v1 installed a third-party PowerShell module from PSGallery
> at runtime, as Administrator, trusting the repository — a supply-chain hole
> and a slow one. The COM API is built into Windows, needs no install, and is
> considerably faster. This is a real dependency reduction, not a refactor.

### 2.2 Restore points
- [x] Implement the `restore-point` config flag (currently parsed, unimplemented).
- [x] `SRSetRestorePoint` via COM; skip gracefully when System Protection is off.
- [x] Create once per run, before the first apply — not per package.

### 2.3 Maintenance actions
- [x] Temp cleanup, Recycle Bin, DISM/SFC health, startup-programs viewer.
- [x] Model as a `Maintenance` trait separate from `Backend` — these are not
      package updates and should not flow through the update policy.

### 2.4 Scheduling
- [x] Task Scheduler (Windows), launchd (macOS), systemd timer (Linux).
- [x] `odysync schedule --daily 09:00` / `odysync unschedule`.

### 2.5 Reporting
- [x] Diagnostics bundle (`odysync diagnostics --out bundle.zip`).
- [x] Text report format alongside the existing JSON.

**Exit criteria:** every v1 flag has a v2 equivalent; `legacy/` deleted.

**Status:** All Phase 2 features implemented. 110 tests pass, clippy-clean.
`legacy/` deletion remains as a follow-up cleanup task.

---

## Phase 3 — GUI (Tauri v2 + React)

**Goal:** the modern, clean desktop app.

### 3.1 Shell
- [x] Tauri v2 workspace member `apps/gui`, reusing `odysync-core` directly —
      the GUI is a front-end over the engine, never a reimplementation.
- [x] React + TypeScript + Vite. Tauri commands wrap `scan` / `plan` / `apply`.
- [x] Streaming progress: backend emits per-package events, UI subscribes.

### 3.2 Design
- [x] Dark/light, system-following. Single accent, generous whitespace, no chrome.
- [x] Update list with per-package version delta, size, and **the skip reason
      shown inline** — the "why didn't this update?" question should never
      require the CLI.
- [x] Holds and pins editable from the UI, writing the same config file.

### 3.3 Elevation model ⚠️ **decide before building**
Store apps must run **unelevated**; drivers must run **elevated**. One process
cannot do both.

Plan: GUI runs unelevated. A short-lived elevated helper is spawned per
privileged batch (UAC prompt once), speaking JSON-RPC over a pipe to the
unelevated parent. Alternative is a persistent elevated service, which is more
convenient and a much larger attack surface. **Recommendation: helper process.**

**Exit criteria:** every CLI capability reachable from the GUI; both share one
config and one policy engine.

**Status:** Phase 3 complete. Tauri v2 + React + TypeScript + Vite + TailwindCSS.
Four tabs (Updates, Maintenance, Schedule, Settings). Dark/light mode. All Rust
and TypeScript compiles clean, clippy clean, 99 tests pass.

---

## Phase 4 — Background operation

- [x] `odysync daemon` — periodic scan, no install without consent.
- [x] Native notification on updates found; deep-link into the GUI.
- [x] Tray icon (Tauri), "scan now", "quit".
- [ ] Idle/AC-power awareness; never scan on battery by default.
- [ ] Memory target: < 15 MB RSS idle. Measure, don't assume.

> Windowless execution already works (`CREATE_NO_WINDOW`, no shell), so this
> phase is scheduling and UX, not a rework of process handling.

---

## Phase 5 — Supply chain & hardening

- [ ] Code-sign Windows binaries (Authenticode) — without this, SmartScreen
      flags every release and users are trained to click through warnings.
- [ ] macOS: sign + notarize + staple.
- [ ] Linux: `.deb`, `.rpm`, AppImage; sign the repo metadata.
- [x] `cargo-audit` + `cargo-deny` in CI; fail on known advisories.
- [x] SBOM (CycloneDX) per release.
- [x] Reproducible builds; publish SHA-256 for every artifact.
- [ ] Self-update path that verifies its own signature before applying.

> Note the asymmetry worth closing: we verify *other* people's installers but
> ship unsigned ourselves. 5.1 and 5.2 fix that.

---

## Phase 6 — Release v2.0.0

- [x] Rewrite `README.md` for v2 (it still documents the Python tool).
- [x] Migration note: v1 profiles → v2 config.
- [x] `CHANGELOG.md` + release notes.
- [ ] Tag `v2.0.0`, merge `dev/rust-rewrite` → `main`, delete `legacy/`.

---

## Known gaps and honest limitations

Worth stating plainly rather than discovering later:

1. **Rollback is not general.** winget cannot uninstall-to-previous. What we
   have is: a restore point before the batch (Windows), refusing to start a
   bad install, and never reinstalling over a working copy. True per-package
   rollback would need a vendor-manifest catalog (the option declined in
   planning) or filesystem snapshots. **Prevention, not undo.**

2. **We trust winget's manifest hashes.** winget verifies installer digests
   against its own manifest internally, but does not expose them, so
   `odysync-verify` cannot independently re-check winget-sourced installers.
   `expected_sha256` is plumbed through and unused for that backend today.
   Closing this needs either a winget API change or our own catalog.

3. **Homebrew cannot pin per-invocation.** `brew upgrade <formula>` goes to
   the newest formula version; we verify convergence afterwards rather than
   pinning up front. Divergence is reported, not prevented.

4. **`is_elevated()` uses a direct token check on Windows.** Fixed in Phase 2 —
   uses `OpenProcessToken` + `GetTokenInformation` via the `windows` crate instead
   of shelling out to `net session`.

5. **Store apps are listed but rarely updatable** from a non-interactive
   context; Microsoft increasingly routes these through the Store app itself.

---

## Suggested order

Phase 2 before Phase 3. Building the GUI first would mean designing screens for
driver updates and maintenance actions that do not exist yet, then redesigning
them once the real data shapes land. Parity first also lets `legacy/` go, which
removes the burden of keeping two implementations alive.

Phase 5.1 (Windows signing) can run in parallel at any point — it is
procurement and CI work, not engineering, and its lead time is external.
