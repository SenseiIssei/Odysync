# Sensei's Updater v2

Safe, fast software and driver updates for Windows, macOS, and Linux.

A complete Rust rewrite of the original Python tool, with a strict safety policy
engine, a CLI, and a Tauri v2 + React desktop GUI.

> Enjoying the updater? Buy me a coffee: **https://ko-fi.com/senseiissei**

---

## Why v2?

The original Python updater had four defects that could corrupt installations:

1. It upgraded packages whose installed version was `Unknown`, regularly
   sidegrading or downgrading them.
2. It compared versions as strings, so `1.10` looked older than `1.9`.
3. On a failed upgrade it fell back to reinstalling from scratch, wiping state.
4. It installed a third-party PowerShell module from PSGallery at runtime as
   Administrator -- a supply-chain hole.

v2 fixes all four with a pure, testable policy engine and the built-in Windows
Update Agent COM API (no third-party modules).

---

## Features

- **Five backends**: winget, Microsoft Store, Windows Drivers (COM API),
  Homebrew, apt, Flatpak
- **Safety policy**: stable-only by default, version comparison via semver,
  holds/pins, exclusions, elevation rules
- **Verification**: installer digest verification and signature checking
  (`sensei-verify` crate)
- **Restore points**: system restore point before applying (Windows)
- **Maintenance**: temp cleanup, recycle bin, DISM/SFC, startup programs
- **Scheduling**: Task Scheduler (Windows), launchd (macOS), systemd (Linux)
- **Diagnostics**: zip bundle for troubleshooting
- **GUI**: Tauri v2 + React + TypeScript + TailwindCSS desktop app
- **Daemon**: background scan mode with system tray

---

## Quick start (CLI)

```sh
# Scan for available updates
sensei scan

# Apply all safe updates
sensei apply --yes

# Hold a package
sensei hold Mozilla.Firefox

# Schedule daily scans
sensei schedule --daily --time 09:00

# Run maintenance
sensei maintain --action temp-cleanup
```

## Quick start (GUI)

```sh
cd apps/gui
npm install
npx tauri dev
```

---

## Architecture

| Crate | Purpose |
|-------|---------|
| `sensei-core` | Policy engine, version algebra, planner, runner, config |
| `sensei-verify` | Installer digest and signature verification |
| `sensei-backends` | winget, msstore, Windows drivers, homebrew, apt, flatpak |
| `sensei-cli` | The `sensei` command-line tool |
| `sensei-gui` | Tauri v2 + React desktop app |

Every safety decision lives in `sensei-core`. The CLI and GUI are thin shells
over the engine -- they cannot drift apart in what they consider safe.

See `ARCHITECTURE.md` for details.

---

## Building

```sh
# CLI only
cargo build --release -p sensei-cli

# GUI (requires Node.js 20+)
cd apps/gui && npm install
npx tauri build
```

## Testing

```sh
cargo test --workspace --lib
cargo clippy --workspace --all-targets
cargo fmt --all -- --check
```

---

## License

MIT

## Migration from v1

v1 profiles map to v2 config `profiles` entries. Holds and pins use the same
`backend:id` syntax. The config file lives at:
- Windows: `%APPDATA%\SenseisUpdater\config.json`
- macOS: `~/Library/Application Support/SenseisUpdater/config.json`
- Linux: `~/.config/senseis-updater/config.json`
