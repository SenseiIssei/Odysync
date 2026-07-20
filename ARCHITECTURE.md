# Architecture — v2 (Rust)

> The v1 Python implementation lives in `legacy/` and still runs. It will be
> removed when v2.0.0 ships.

## Why the rewrite

v1 could damage a working installation. Three defects combined to do it:

1. **Reinstall-on-failure.** When `winget upgrade` failed, v1 ran
   `winget install --id <pkg>`, reinstalling from scratch over a working copy.
   This is what forced manual repair of apps.
2. **String version comparison.** `1.10.0` sorted below `1.9.0`, so downgrades
   were presented as upgrades.
3. **`--include-unknown`.** Packages whose installed version winget could not
   read were upgraded anyway, regularly sidegrading them.

v2 makes each of these impossible by construction rather than by convention.

## Crate layout

The workspace is deliberately layered so that safety rules cannot be bypassed
by a backend, and so the CLI and the (Phase 2) Tauri GUI share one brain.

```
odysync-core       domain model, version algebra, safety policy, planner, runner
  └─ no knowledge of any specific package manager; pure and fully unit-tested

odysync-verify     SHA-256 integrity + platform code-signature verification
  └─ Authenticode (Windows), codesign (macOS), repo-signed (Linux)

odysync-backends   package manager integrations
  └─ winget, msstore, homebrew, apt, flatpak

odysync-cli        `odysync` binary — argument parsing and rendering only
```

Dependencies point one direction only: `cli -> backends -> core`. A backend
cannot reach into the CLI, and `core` cannot reach into a backend.

## Where each guarantee lives

| Guarantee | Enforced in | How |
|---|---|---|
| Never downgrade or sidegrade | `core::version`, `core::policy` | Segment-wise numeric comparison; `is_upgrade_to` is the only path to "yes" |
| Never act on an unknown version | `core::policy` | `require_known_versions`; `Version::compare` returns `None`, never `Equal` |
| Never reinstall over a working copy | `backends::*::apply` | No fallback path exists. A failed upgrade returns `Err` and stops |
| Always install the approved version | `backends::*::apply` | `--version <exact>` / `pkg=version`; refuses to run without one |
| Confirm the update landed | `core::runner` | Reads the version back; exit code 0 alone is not accepted as success |
| No console windows | `core::proc` | `CREATE_NO_WINDOW`, no shell, `--disable-interactivity` |
| Nothing hangs forever | `core::proc` | Every spawn is time-boxed and `kill_on_drop` |
| Stable channel only | `core::policy` | `stable_only` rejects alpha/beta/rc/nightly |
| Store apps never run elevated | `core::model`, `core::policy` | `forbids_elevation()` is a hard block |

Because `core` performs no I/O, every rule above is covered by unit tests that
run on all three platforms without needing the package manager installed.

## The version algebra

`core::version::Version` is the foundation. It parses the messy reality of
package versions (`1.2.3.4`, `2024.05.01`, `1.0.0-rc.2`, `17.14.29 (March 2026)`,
`Unknown`) into ordered segments.

The critical design decision: **an unparseable version is `Unknown`, and
comparing anything against `Unknown` returns `None` — never `Equal`.** Callers
must treat `None` as "refuse to act". This makes "do nothing" the default answer
whenever the tool is uncertain, which is the opposite of v1's behaviour.

## winget table parsing

winget has no machine-readable output for `upgrade`, so the table is parsed.
Two properties make this robust where v1 was not:

- **Column offsets, measured in terminal display width**, are taken from the
  header row. v1 split on runs of two-or-more spaces, so an app name containing
  a double space shifted every later field left — a *version* string could be
  read as the package **Id**. Display width (not chars, not bytes) is required
  because winget pads to rendered width, and one CJK glyph is two columns wide.
- **Position, not header text.** Column order is fixed across locales even
  though the labels are translated, so a German or Japanese install parses
  correctly. Verified against real German winget output.

Trailing prose is rejected by validating the Id's shape (ASCII, unspaced, no
leading/trailing dot). Localised summary text sliced at a column boundary can
otherwise look plausible — German winget yields the fragment `erfügbar.`, which
has no whitespace and even contains a dot.

## Adding a backend

Implement `core::backend::Backend` (five methods) and add one line to
`backends::all_backends()`. Nothing else changes: policy, planning,
verification, reporting and both front-ends pick it up automatically.

The trait's contract is documented in `core/src/backend.rs` and is binding —
in particular, `apply` must pin an exact version and must never fall back to
installing a package that is already present.

## Status

Phase 1 (core engine + CLI) is complete: 100 unit tests, clippy-clean,
warning-free, CI on Windows/macOS/Linux.

Phase 2 (Tauri + React GUI, background service) reuses these crates unchanged;
the GUI is a front-end over `core`, not a reimplementation.
