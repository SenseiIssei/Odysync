# Architecture

> The v1 Python implementation was removed once the Rust CLI reached feature
> parity. It remains available at the `v1.3.0` tag.

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
odysync-core       domain model, version rules, safety policy, planner, runner
  └─ no knowledge of any specific package manager; unit-tested everywhere

odysync-verify     SHA-256 integrity + platform code-signature verification
  └─ Authenticode (Windows), codesign (macOS), repo-signed (Linux)

odysync-backends   every update source and every maintenance tool
  ├─ direct/       installers straight from the publisher (catalog, Sparkle)
  ├─ windows_update/, gpu/, fwupd: drivers, firmware and the OS itself
  ├─ guard, scan: what every source's apply and every scan go through
  ├─ inactive: sources known by name that cannot keep the contract, and why
  ├─ homebrew, apt, dnf, pacman, zypper, flatpak, snap, nix, …
  ├─ cleanup, components, devices, startup_items, system_info
  └─ winget, chocolatey, scoop: optional

odysync-cli        `odysync` binary — argument parsing and rendering only
apps/gui           Tauri 2 shell + React front-end over the same crates
```

Dependencies point one direction only: `cli/gui -> backends -> core`. A
backend cannot reach into a front-end, and `core` cannot reach into a backend.

## Where each guarantee lives

| Guarantee | Enforced in | How |
|---|---|---|
| Never downgrade or sidegrade | `core::version`, `core::policy` | Segment-wise numeric comparison; `is_upgrade_to` is the only path to "yes" |
| Never act on an unknown version | `core::policy` | `require_known_versions`; `Version::compare` returns `None`, never `Equal` |
| Never reinstall over a working copy | `backends::*::apply` | No fallback path exists. A failed upgrade returns `Err` and stops |
| Always install the approved version | `backends::*::apply` | `--version <exact>` / `pkg=version`; refuses to run without one |
| Confirm the update landed | `core::runner` | Reads the version back; exit code 0 alone is not accepted as success |
| No console windows | `core::proc` | `CREATE_NO_WINDOW`, no shell, `--disable-interactivity` |
| Nothing hangs forever | `core::proc`, `backends::scan` | Every spawn is time-boxed and ends with everything it started (a job object on Windows, a session of its own on Unix); every source has its own scan limit; Windows Update runs on a thread that can be left behind |
| An installer never runs twice | `core::runner`, `core::error`, `core::proc` | A timeout is never retried; the installed version is read back instead. `run_installer` waits for what the installer started |
| Never over something newer | `backends::guard`, every `apply` | What is installed is read again and must still be older |
| One run at a time | `core::apply_lock` | An operating-system lock held by app, terminal or schedule |
| Room on the disk | `core::health`, `core::runner` | Checked before every install |
| Windows' own tools are Windows' own | `core::proc` | Full path from `GetSystemDirectoryW`, not from `PATH` |
| Nothing planted next to an elevated installer | `backends::direct::private_dir` | A fresh directory with an administrators-only access list |
| Unattended runs leave drivers alone | `core::model::in_unattended_care` | Used by `care`, `apply --unattended`, the daemon and schedules |
| Stable channel only | `core::policy` | `stable_only` rejects alpha/beta/rc/nightly |
| Store apps never run elevated | `core::model`, `core::policy` | `forbids_elevation()` is a hard block |

The policy itself performs no I/O, so every rule is covered by unit tests that
run on all three platforms without needing the package manager installed.

## The version algebra

`core::version::Version` is the foundation. It parses the messy reality of
package versions (`1.2.3.4`, `2024.05.01`, `1.0.0-rc.2`, `17.14.29 (March 2026)`,
`Unknown`) into ordered segments.

The critical design decision: **an unparseable version is `Unknown`, and
comparing anything against `Unknown` returns `None` — never `Equal`.** Callers
must treat `None` as "refuse to act". This makes "do nothing" the default answer
whenever the tool is uncertain, which is the opposite of v1's behaviour. The
same goes for two versions whose difference has no agreed meaning, such as
`1.0.0` against `1.0.0-4`.

Distribution packages follow rules of their own. `1.1.1w-0+deb11u1` to
`+deb11u2` is a security update, and the general rules read it as the same
version. `core::version_native` implements dpkg's, rpm's and pacman's ordering
and PEP 440, tested with the reference cases of each; `Version::parse_as`
selects one, and versions under different rules are never compared.

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

## Installers Odysync downloads itself

Most sources hand the whole job to a tool that verifies its own payload —
apt, Homebrew, the Windows Update Agent. Three do not: the `direct` source,
the NVIDIA driver source and the runtime components download an installer
and run it. For those the chain lives in
`backends::direct::windows_apply::lock_and_verify` and is the same for all
three:

1. the release is re-read and must equal the approved version;
2. the URL must belong to the publisher — a GitHub asset must be a release of
   the repository the catalog names, a vendor URL must start with the pinned
   prefix — and every redirect must stay on HTTPS;
3. the download directory is new, and when Odysync is elevated only
   administrators can write to it (a library next to an installer is loaded
   with its rights);
4. the file is opened with `FILE_SHARE_READ` only, so nothing can write,
   rename or delete it until the installer exits, and it is hashed from disk
   under that lock;
5. the published digest must match;
6. the Authenticode signature must be valid and its subject's CN or O must
   equal the signer the catalog pins. The subject is parsed, not searched, so
   a quoted comma or a longer name cannot satisfy the check.

The catalog (`backends/src/direct/catalog.json`) only lists apps whose
installers are signed; each signer was read from the real installer. On macOS
the trust anchor is the installed app itself: its Sparkle Ed25519 key and its
Apple Team ID.

## Adding a backend

Implement `core::backend::Backend` (six methods) and add one line to
`backends::all_backends()`. Nothing else changes: policy, planning,
verification, reporting and both front-ends pick it up automatically.

The trait's contract is documented in `core/src/backend.rs` and is binding —
in particular, `apply` must pin an exact version and must never fall back to
installing a package that is already present. `backends::guard` has the checks
every `apply` starts with. A tool that cannot install one exact item belongs in
`backends::inactive`, with the reason.

## Status

What is verified, how, and what is not is kept in [NEXT.md](NEXT.md); what comes
next in [ROADMAP.md](ROADMAP.md).
