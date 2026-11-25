# Odysync: the plan

> Status: 2026-11-25. Phases 1 to 6 below are the Rust rewrite, all shipped.
> Part A is what comes next, ordered by value against effort. What is verified
> and what is not is in [NEXT.md](NEXT.md), what changed in the
> [CHANGELOG](CHANGELOG.md).

Each item says why, what done looks like, how to check it, and who has to act:
**[owner]** needs a decision or an account, **[money]** costs something,
**[people]** needs other hardware or other people. Everything else can simply
be done. Effort: S is days, M one to two weeks, L more.

---

# Part A. From here

## Where Odysync stands, honestly

Checked on 2026-11-25. "?" means it could not be confirmed.

| Tool | State | Coverage | Verifies | Rolls back | Better than Odysync today |
|---|---|---|---|---|---|
| [UniGetUI](https://github.com/Devolutions/UniGetUI) | 2026.3.4, 13 Nov; owned by Devolutions since March | every package manager on three systems | manager's hash; [warns when the host changes](https://docs.devolutions.net/unigetui/guides/updates/update-safely) | package list only | breadth, about 60 languages, minimum age, portable apps |
| [Patch My PC Home](https://patchmypc.com/home-updater-release-notes) | 5.4.5 (July; newer ?) | about 570 apps | ? | restore point | catalogue, portable apps, German, parallel downloads |
| [Ninite](https://ninite.com/) | current | 150 apps | "signatures or hashes" | no | no questions asked at all |
| [Topgrade](https://github.com/topgrade-rs/topgrade) | 17.17.0, 17 Nov | runs every local manager | delegated | no | number of steps, weekly releases |
| [Winget-AutoUpdate](https://github.com/Romanitho/Winget-AutoUpdate) | 2.12.0, May | winget | winget's hash | no | Intune and group policy |
| [IObit Software Updater](https://www.iobit.com/en/iobit-software-updater.php) | 9.2, two updates a day free | "1,100+" | claims only | restore point | catalogue size |
| [Homebrew 7](https://www.helpnetsecurity.com/2026/09/15/homebrew-7-0-0/) | September | brew | build attestations, sandboxed casks, OSV | partly | supply-chain controls |
| [Floodtide](https://www.ifun.de/floodtide-neue-macupdater-alternative-laesst-sich-ausprobieren-288094/) (macOS, paid) | September | about 3,200 apps | Developer ID must match | seven days | macOS coverage, rollback, German |
| [duo-updater](https://github.com/jizhi0v0/duo-updater) (macOS, MIT) | 0.5.30, 24 Nov | Sparkle, App Store, brew, GitHub | signatures | yes | the same idea, with rollback |
| [Windows app updates](https://www.windowslatest.com/2026/11/09/for-20-years-windows-apps-like-java-have-nagged-you-with-their-own-update-pop-ups-and-windows-11-now-wants-to-end-it/) | in test, no providers yet | apps that register | vendor | no | it will be part of Windows |

[SUMo](https://alternativeto.net/news/2023/9/kc-softwares-announces-shutdown-ceasing-sales-of-the-popular-sumo-app-and-many-others)
ended in 2023 and [MacUpdater](https://tidbits.com/2026/01/09/macupdater-shuts-down-leaving-users-searching-for-alternatives/)
on 1 January 2026, so their users are looking.

**What Odysync has that none of them shows:** an exact approved version, the
publisher's own source, and a pinned signer, all three required. That is not
academic: the [Notepad++ update server was hijacked](https://helpnetsecurity.com/2026/02/18/notepad-secure-update-download)
from June to December 2025, and its own updater did not verify what it
downloaded. Odysync's pin refuses exactly that.

**What it lacks:** coverage (15 apps in the catalogue; on the development
machine 6 of 119 installed programs are recognised), rollback, signed releases,
any language but English, portable apps, and until this release proof that its
own installs work: Windows Update had been reporting success as failure since
2.1 and no test could notice.

## The order

| # | What | Value | Effort | Needs |
|---|---|---|---|---|
| 1 | Every install path exercised by a machine | highest | M | nothing |
| 2 | Signed releases, then self-update | highest | S + waiting | **[owner]**, maybe **[money]** |
| 3 | Catalogue from 15 to 150, drafted by a bot, pinned by a check | highest | M | review time |
| 4 | Undo for an update | high | M to L | nothing |
| 5 | Fixes first: exploited holes on top, a waiting period for the rest | high | S to M | nothing |
| 6 | One elevation, open programs handled, downloads first | high | M | a decision on the helper |
| 7 | A proof page and a stop when the publisher changes | high | S | nothing |
| 8 | German, and the means for more | medium | M | **[owner]** reads it |
| 9 | Automatic care that is really unattended | medium | S to M | 2 and 6 for the full form |
| 10 | Where people find it: winget, Scoop, Homebrew | medium | S | **[owner]** publishes |
| 11 | macOS beyond Sparkle | medium | M | **[people]**, **[money]** |
| 12 | Portable apps, duplicates, per-user copies | medium | M | nothing |
| 13 | Hardware makers' tools, or an honest "no" | medium | L | **[people]** |
| 14 | Faster still | low | S | nothing |
| 15 | Housekeeping of the project itself | low | S | **[owner]** for GitHub settings |

---

## A1. Every install path exercised by a machine

**Why.** `orcFailed` was taken to be 2. It is 4; 2 is success. For four releases
every successful driver install, and since 3.0 every security update, was
reported as failed, all tests were green, and NEXT.md said openly that the path
had never been run. About a dozen other
sources could not have worked by their own code, for the same reason: nobody
had ever run them. The lesson is not "more unit tests".
GitHub's runners are elevated, real and thrown away afterwards; that is the
machine this project was missing.

**Done when**
- [ ] A weekly job on Windows installs the *previous* release of each catalogue
      app, runs `odysync apply --only <app> --yes`, and reads the new version
      from the registry. One job per installer kind (Inno, NSIS, MSI) on every
      pull request that touches `direct/`.
- [ ] The same job installs one pending Defender definition through the Update
      Agent and checks the result code path end to end.
- [ ] On Linux, containers of Debian, Fedora, openSUSE and Arch with one package
      held at an older version: `odysync apply` brings it to exactly the offered
      one. Real output of each manager is kept as a fixture.
- [ ] On macOS, an old Rectangle is updated through Sparkle.
- [ ] A negative run: a tampered installer, a wrong signer, a moved release and
      a running program are each refused, on the real path.
- [ ] NEXT.md's table lists the job that proves each row, not a memory of it.

**Verify.** Break the result code again on a branch; the job must go red.

## A2. Signed releases, then self-update **[owner]** maybe **[money]**

**Why.** An unsigned updater teaches people to click past the warning that
protects them, and Smart App Control blocks it outright with
[no way round per app](https://support.microsoft.com/lt-LT/Windows/Security/threat-malware-protection/smart-app-control-frequently-asked-questions).
It also keeps self-update switched off.

**What changed.** Azure Trusted Signing (now Artifact Signing), which
docs/SIGNING.md recommended, is
[open to individuals in the US and Canada only](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart);
organisations in the EU qualify. EV certificates no longer skip SmartScreen.

**Options, in this order**
1. [SignPath Foundation](https://signpath.org/terms): free for open source.
   Wants an OSI licence, builds from the repository, two-factor sign-in, manual
   approval per release, a code-signing policy page and a project with some
   reputation. Risks: three stars, and the security audit must be described as
   what it is, checks of the local configuration, since tools that exploit
   vulnerabilities are excluded.
2. [Certum Open Source](https://shop.certum.eu/open-source-code-signing-on-simplysign.html):
   49 euros, in the cloud. Whether CI can sign unattended is not confirmed.
3. A registered business (Gewerbe) for the organisation route at Microsoft.
   Not confirmed that a sole trader counts.
4. macOS: [Apple Developer Program](https://developer.apple.com/programs/whats-included/),
   99 dollars a year. Homebrew refuses casks that Gatekeeper refuses since
   September.

**Done when**
- [ ] `Get-AuthenticodeSignature` says Valid for the installer *and* for
      `odysync.exe` and `odysync-gui.exe` inside it. The release workflow signs
      only the finished installer today; Tauri's `signCommand` signs the parts.
- [ ] A clean Windows 11 with Smart App Control installs it without a warning.
- [ ] `self_update.rs` installs a signed release through
      `direct::lock_and_verify` with Odysync's own signer pinned, keeps the old
      version until the new one has started once, and the refusing test is
      replaced by one for the signer.

**Verify.** A release candidate updates an installed 3.x on a throw-away
machine; a copy re-signed by anyone else is refused.

## A3. Catalogue from 15 to 150

**Why.** Coverage is the first thing anyone compares. Six of 119 programs on a
developer's machine is not enough to replace anything.

**How.** Public data drafts, pins decide. `winget-pkgs` (MIT) has the URL, the
SHA-256, silent switches and the product code; Scoop buckets (Unlicense) and
Homebrew casks (BSD) have version rules; GitHub publishes a
[digest per release file](https://github.blog/changelog/2025-06-03-releases-now-expose-digests-for-release-assets/).
None of them proves who made a file. The signer read from the real installer
does, and it stays the gate.

**Done when**
- [ ] A script drafts an entry: source, asset pattern, installer kind, uninstall
      match; downloads the installer in CI, reads its signature, checks the
      hash against two sources, and opens a pull request with the evidence.
- [ ] 50 entries within a month, chosen by how many people have them (browsers,
      runtimes, chat, media, archivers, editors), 150 within a quarter.
- [ ] More than one source kind: redirecting "latest" URLs that are verified by
      signer and file version instead of failing on a stale hash
      ([242 open winget reports](https://github.com/microsoft/winget-pkgs/issues?q=is%3Aissue+hash+in%3Atitle)
      are about that), Mozilla's and Google's version feeds, plain JSON.
- [ ] The catalogue ships on its own: signed with a key embedded in the app,
      never older than the built-in one, fetched at most daily, and a wrong
      signature means the built-in one is used.
- [ ] Entries are exercised by A1's weekly job. One that stops resolving or
      changes signer opens an issue by itself (the resolving half exists).
- [ ] Packages that record a file version get it compared with the approved one
      before they run.

**Verify.** On a fresh Windows with the 50 most-installed free programs, the
scan recognises at least 40.

## A4. Undo for an update

**Why.** "Never break a working machine" without a way back is prevention only.
On macOS two newer tools keep the old version for days; on Windows none of the
reviewed ones does it per program.

**Done when**
- [ ] `odysync undo <app>` and a button in History, for seven days.
- [ ] macOS: the replaced bundle is kept instead of deleted (it already exists
      until the swap is done).
- [ ] Windows, per-user programs: the program's folder and its uninstall entry
      are copied before the update (hard links on the same volume, so it costs
      little) and put back as one step.
- [ ] Windows, machine-wide: the previous installer is downloaded and verified
      *before* the update when the publisher still offers it; where a product
      refuses to go back, History says so beforehand instead of offering it.
- [ ] Linux: the exact previous version is named, with the command that installs
      it; dnf and Nix use their own undo.
- [ ] The space used is shown and capped; nothing is kept for drivers, which
      have their own backup.

**Verify.** A1's job updates, undoes and reads the old version back, one job
per installer kind.

## A5. Fixes first, the rest can wait

**Why.** "14 updates" tells nobody which one matters tonight. CISA's
[known exploited vulnerabilities](https://www.cisa.gov/known-exploited-vulnerabilities-catalog)
are CC0, 1,796 entries on 2026-11-25; no reviewed updater shows them. The other
half is the opposite wish: let a release be a few days old before it lands,
which UniGetUI has and Topgrade users
[ask for](https://github.com/topgrade-rs/topgrade/issues/1890).

**Done when**
- [ ] Catalogue entries carry a vendor and product id; an update that fixes an
      exploited hole is marked, sorted first, and named with its CVE.
- [ ] `minimum-age` in days per source, default 0 for security sources and 3 for
      the rest of automatic care; an exploited hole overrides it.
- [ ] Programs out of support are named ([endoflife.date](https://endoflife.date/docs/api/v1/), MIT).
- [ ] Offline, the last list is used and its date shown.

**Verify.** A fixture machine with one vulnerable and one current program; a
release published an hour ago is held with a reason, the same with a KEV entry
is not.

## A6. One elevation, open programs, downloads first

**Why.** Installing a machine-wide program means restarting the whole app as
administrator, after which Store and Scoop are refused. UniGetUI users complain
about [repeated prompts](https://github.com/Devolutions/UniGetUI/issues/3670)
and want [everything downloaded before anything installs](https://github.com/Devolutions/UniGetUI/issues/4533).
Phase 3.3 below recommended a helper process and it was never built.

**Done when**
- [ ] The app stays unelevated. One prompt starts a helper that accepts only
      *which catalogue entry, which scope, which version*: never a path, a URL
      or a command line. It downloads and verifies by itself, in its own
      protected directory (exists), and exits after the batch.
- [ ] All downloads finish and verify before the first install starts; progress
      is bytes, not "0 of 3" (`apply_with_progress` has no caller today).
- [ ] An open program is offered as *close and update*, through the
      [Restart Manager](https://learn.microsoft.com/en-us/windows/win32/rstmgr/about-restart-manager),
      or *when I close it*, which waits. Never closed unasked.
- [ ] Per-user installs never ask for rights.

**Verify.** Three programs, one per-user, two machine-wide, one of them open:
one prompt, no restart of the app, the open one updated after it is closed.

**Decide first** (owner): helper per batch, as recommended, or a service.

## A7. A proof page, and a stop when the publisher changes

**Why.** "Verified" should be checkable. Odysync knows source, hash, signer and
log of every install and shows none of it afterwards.

**Done when**
- [ ] History shows per update: the URL, both hashes, the signer, the time, the
      exit code, the version read back, and a copy button.
- [ ] A signer different from the one seen last time is refused with both names,
      also where the catalogue would allow it, until somebody agrees. On macOS
      the same for the Team ID, on winget and Scoop for the host.
- [ ] Odysync's own releases: attestations (in the workflow now),
      [immutable releases](https://github.blog/changelog/2025-10-28-immutable-releases-are-now-generally-available/),
      the SBOM attached to the release instead of only to the CI run.

**Verify.** Two fixture installers with different valid signers; the second is
refused and the proof page shows why.

## A8. German, and the means for more

**Why.** The owner and many of the people who would try it are German; every
German competitor and Patch My PC speak it.

**Done when**
- [ ] Every string of app and CLI comes from one message file per language; a
      test fails on a missing or unused key; dates, sizes and plurals go through
      the platform.
- [ ] German complete and read by a person **[owner]**; the language follows the
      system and can be changed in Settings.
- [ ] Messages that come from a program stay verbatim.

**Verify.** A test renders every page in both languages and finds no English in
the German one apart from names.

## A9. Automatic care that is really unattended

**Why.** It works now without drivers and pip, survives a missed start, and is
not killed by a pulled plug. What is left:

- [ ] No console window when the schedule runs (a windowless build of the CLI,
      or the app's own binary with a verb).
- [ ] A report next time somebody signs in: what was updated, what waits, why.
- [ ] Machine-wide programs without anybody present: a task with rights,
      registered once with consent, that runs only a signed Odysync from a
      directory ordinary users cannot write to. Not before A2.
- [ ] Linux: a system timer for apt, dnf and zypper through polkit.
- [ ] Not while a game or a presentation is in full screen
      (`SHQueryUserNotificationState`), not on a metered connection.
- [ ] When [Windows' own orchestration](https://www.windowslatest.com/2026/11/09/for-20-years-windows-apps-like-java-have-nagged-you-with-their-own-update-pop-ups-and-windows-11-now-wants-to-end-it/)
      opens to third parties: register as a provider instead of competing.

**Verify.** CI registers, starts and reads back the task (registering exists).

## A10. Where people find it **[owner]**

- [ ] winget-pkgs: needs a silent install and a versioned URL, both there, and
      no antivirus flags. No signature rule was found, so it can go now. Each
      submission is a public act and waits for a yes.
- [ ] An own Scoop bucket and an own Homebrew tap: no star thresholds, updated
      by the release workflow. Scoop Extras wants
      [100 stars](https://github.com/ScoopInstaller/Extras/blob/master/.github/ISSUE_TEMPLATE/package-request.yml),
      Homebrew three times the usual for one's own project.
- [ ] Not Flathub: system tools are
      [excluded](https://docs.flathub.org/docs/for-app-authors/requirements).
- [ ] A short comparison page with the table above, kept honest by date, and a
      post to the places where MacUpdater and SUMo users ask.

**Verify.** `winget install Odysync` and `scoop install odysync` on a clean
machine.

## A11. macOS beyond Sparkle **[people]** **[money]**

- [ ] Run the existing path on a real Mac: the first thing to do there.
- [ ] Apps without Sparkle: match installed bundles to Homebrew's cask data
      (BSD), require the installed app's Team ID, download from the publisher.
- [ ] App Store apps listed through `mas`, updated by the store.
- [ ] GUI processes started from Finder see `/opt/homebrew/bin`.
- [ ] Signed and notarised (A2).

**Verify.** The macOS job of A1 plus one person with a Mac for a week.

## A12. Portable apps, duplicates, per-user copies

- [ ] Duplicates recognised by UpgradeCode, shown once with both locations.
- [ ] Portable programs in folders the user names: version from the file, the
      same source and signer rules, replaced as a folder with undo (A4).
- [ ] x86 and x64 of the same program side by side.

**Verify.** Fixture folders; the same app installed twice appears once.

## A13. Hardware makers' tools, or an honest "no" **[people]**

Dell, HP and Lenovo tools install sets, not single approved items, and their
output was never seen here. Windows Update carries their drivers already.

- [ ] One volunteer with each machine runs `odysync diagnostics`; real output
      becomes a fixture.
- [ ] A source returns only when it can install exactly one approved item and
      read the result back. Otherwise the app links to the maker's own tool.
- [ ] AMD and Intel graphics: no machine-readable feed was found; say so.

## A14. Faster still

A full scan on the development machine took 31 to 45 s at the start of this
work and 11 to 24 s now; the rest is pip and Windows Update.

- [ ] Show results per source as they arrive, not only their progress.
- [ ] Ask sources whose state cannot have changed less often (a file time of
      the package database says so).
- [ ] Windows Update asks its local cache first and refreshes behind it.
- [ ] Target: first useful answer in under 2 s, complete in under 10 s.

## A15. Housekeeping of the project **[owner]**

- [ ] Allow Actions to open pull requests, or the weekly lockfile refresh ends
      in an issue with a link instead of a merge.
- [ ] Delete `chore/lockfiles-2026-*` (four), `dev/*`, `feat/smart-care`,
      `backup/*`; tag `dev/ms3` first (three commits no tag contains).
- [ ] Pin actions by commit; Dependabot keeps them current.
- [ ] One end-to-end test per Tauri command (`tauri-driver`), so that a denied
      permission such as the maximize button shows up in CI.
- [ ] Finish or remove the offline cache: nothing fills it. Download now and
      install later is the useful form and comes with A6.
- [ ] Generate the TypeScript types from Rust; they are kept in step by hand.

---

# Part B. The Rust rewrite, phases 1 to 6

> History. All of it shipped; what was left open has moved to Part A.

Each phase is independently shippable. Nothing below requires a rewrite of
anything above it: the crate boundaries from Phase 1 are what make that true.

## Phase 1. Core engine and CLI: done

The safety policy, version algebra, planner, runner, verification crate, five
backends (winget, msstore, homebrew, apt, flatpak), and the `odysync` CLI. The
four defects of v1 that corrupted installs are fixed and covered by regression
tests; see `ARCHITECTURE.md` for where each guarantee is enforced.

## Phase 2. Windows feature parity: done

Drivers through the Windows Update Agent instead of a PowerShell module fetched
at runtime as Administrator, restore points, maintenance actions, scheduling on
three systems, diagnostics and reports. `legacy/` is deleted; the Python tool
remains at the `v1.3.0` tag.

## Phase 3. The app: done

Tauri 2 and React over the same engine, dark and light, the reason an update is
held back next to it.

**3.3 Elevation model: decided, not built.** Store apps must run unelevated and
drivers elevated, and one process cannot do both. Recommended: the app runs
unelevated and starts a short-lived elevated helper per batch, speaking over a
pipe. The alternative is a persistent elevated service, which is more convenient
and a much larger attack surface. Now A6.

## Phase 4. Background operation: done, with two notes

- [x] `odysync daemon`, tray icon, notifications when a background scan finds
      something, no scans on battery.
- [x] No console windows from the app (`CREATE_NO_WINDOW`).
- [ ] Under 15 MB resident when idle: the daemon logs its size, nobody has
      tracked it.

## Phase 5. Supply chain and hardening

- [ ] Code-sign Windows binaries; sign, notarise and staple on macOS: A2.
- [x] `.deb`, `.rpm`, AppImage. [ ] A signed repository for them.
- [x] `cargo audit`, `cargo deny`, SBOM, SHA-256 and provenance attestation for
      every release file.
- [ ] Self-update that verifies its own signature: A2.

> The asymmetry worth closing: other people's installers are verified here
> before they run, and Odysync ships unsigned itself.

## Phase 6. Release: done

Shipped as v2.1.0 and, with updates straight from the publisher, as v3.0.0.

---

## Known gaps and honest limitations

1. **Rollback is not general.** What there is: a restore point before the batch
   (Windows), refusing to start a bad install, never reinstalling over a working
   copy. Prevention, not undo, until A4.
2. **winget's manifest hashes are trusted.** winget checks installers against
   its own manifest and does not expose the digests.
3. **Some managers cannot pin.** Where a tool always installs its newest, the
   target is asked again right before and the install is refused if it moved;
   the moment in between stays.
4. **Store apps are listed but rarely updatable** without the Store itself.
5. **A hung Windows Update cannot be interrupted.** Odysync stops waiting for it
   and says so; Windows may still be working.
