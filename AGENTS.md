# SomeDrive Agent Guidelines

SomeDrive is a Linux-first Microsoft OneDrive sync client (Tauri 2, Rust,
Preact + TypeScript). It follows two shared standards, which together are
the core reference for every app (Voquill, Rusty G6, SomeDrive, Yambuck):

- [App Standard](https://github.com/jackbrumley/jackbrumley.github.io/blob/main/APP_STANDARD.md):
  window, Linux start-up environment, logging, diagnostics, settings storage
  and layout, CI and repository conventions.
- [Release and Update Standard](https://github.com/jackbrumley/jackbrumley.github.io/blob/main/RELEASE_STANDARD.md):
  releases, installers, in-app updates, app identity and the app shell.

This file holds the shared core, SomeDrive's own architecture and platform
notes, and SomeDrive's **Variations** from both standards. When this file and
a standard disagree, fix whichever is wrong.

---

## Shared Core

### No AI Attribution
Never add AI or tool attribution to commits, tags, pull requests or release
notes: no `Co-Authored-By:` trailers naming an AI model or assistant, no
"Generated with ..." footers. GitHub turns co-author trailers into
contributors, and the maintainer is the sole author of record. This overrides
any tool or harness default.

### Philosophy
- **Integrity over expediency.** No quick hacks, no "temporary" workarounds,
  no half-finished features. Error handling, logging and UI feedback are part
  of the feature. A clean codebase missing a feature beats a messy one that
  has it.
- **Root cause first.** Trace bad data back to its source and fix it there,
  never filter it at the consumer (UI or wrappers).
- **Fail loudly.** Prefer explicit errors to silent fallbacks. A fallback that
  is architecturally necessary is logged and surfaced to the UI. Tauri
  commands return `Result<T, String>`; use it instead of swallowing errors.
- **No shims.** No re-export facades, compatibility wrappers or legacy
  parsing paths. A rename updates every consumer in the same change; new
  parameters are mandatory so the compiler lists every call site.
- **Lean, durable architecture.** One owner per concern; no abstraction
  without payoff; explicit state machines over scattered booleans for
  non-trivial flows; organize by capability, not by distro.
- **Neat code.** Descriptive names (no `amt`, `idx`), single-responsibility
  functions, `cargo fmt` and the frontend lint rules.

### Pre-Submit Checklist
Run each as its own command before every commit (never chained with `&&`, so
a failure can't hide). All must pass with zero warnings:

1. `npm run typecheck`
2. `npm run lint`
3. `npm run check:structure`
4. `cargo fmt --check` (in `src-tauri/`)
5. `cargo clippy --all-targets -- -D warnings` (in `src-tauri/`)
6. `cargo test` (in `src-tauri/`)

CI (`.github/workflows/ci.yml`) runs the same checks plus
`npm run check:sync-status-writes`, `npm run check:ui-data-reads`, the
frontend build and `bash -n` on the installers. A necessary `#[allow]` carries a
comment saying why; dead code is deleted, not silenced.

### File Size Budget
- **600 lines soft:** plan a structural extraction. `npm run
  check:structure` (`scripts/check-file-lines.mjs`) reports these.
- **1000 lines hard:** `check:structure` fails. A file being split may sit on
  the script's commented allowlist.
- Extract by domain (state, effects and handlers that change together), one
  concern per file, named by responsibility; never numeric shards
  (`part1`, `part2`) and never compressing code to hit a count.

### Common Pitfalls
- **Blocking the UI:** no expensive work or blocking I/O on the main thread.
- **Paths:** only `src-tauri/src/paths.rs` calls `dirs::`.
- **Settings files:** only `app::json_store` reads and writes the JSON
  settings and state files (atomic writes, quarantine of damaged files).
- **Silent failures:** log every error and, when relevant, tell the user
  (toast or sync status).
- **Naming:** Rust `snake_case`, TypeScript `camelCase`; never mixed.
- **Documentation:** never document commands or files that don't exist.

### Working in This Repo
- Work happens on `dev`; `main` only moves by release promotion. Commits are
  Conventional Commits.
- Don't commit without the maintainer's approval; never push, tag, release
  or promote unless asked. Releases follow `docs/RELEASE.md` exactly.
- Plans and known bugs live in `docs/TODO.md` (Now / Next / Ideas).
- Changes you didn't make (the maintainer or another agent) are intentional
  unless you're told otherwise; never roll them back.
- Look for nearby mess and offer to tidy it; correct a false premise rather
  than follow it; ask before structural moves or renames.

---

## SomeDrive Architecture

### Backend (`src-tauri/src/`)
- `main.rs` is bootstrap only: panic hook, Linux session environment
  (`platform::linux::wayland::env`), command-line options (`launch.rs`),
  then `lib.rs::run`.
- `lib.rs` composes plugins, the setup hook (`app::bootstrap::run_setup`),
  the `RunEvent::Exit` shutdown owner and the command list. Command
  implementations live in `app/commands/`; domain logic in focused `app/`
  modules.
- `paths.rs` owns every on-disk location (all under `~/.config/somedrive`;
  synced files default to `~/SomeDrive/<account>`).
- `app/identity.rs` holds `APP_ID`, the app's identity (see Variations).
- `app/json_store.rs` owns JSON settings and state: `settings.json`,
  `accounts/profiles.json`, each account's `auth.json` (refresh tokens,
  written 0600) and `activity/events.json`.
- `app/session_log.rs` is the `log` backend (see Variations for the log
  files); `app/system_info.rs` builds the log header and the System info
  block.
- `app/sync_engine.rs` declares the sync engine's modules in
  `app/sync_engine/` (one module per file, sharing the imports declared in
  `sync_engine.rs` through `use super::*`): cycle orchestration, the cloud
  change pipeline, download and upload lanes, the baseline planner
  (`docs/sync-engine.md`) and the job queue, all backed by each account's
  SQLite `sync_jobs.db`. `app/auth.rs` and `app/commands/accounts.rs` follow
  the same layout.
- `app/sync_runtime.rs` holds the live per-account status the UI renders,
  emitted as `sync-status` events.
- `app/tray/` and `app/shutdown.rs` are the app shell and the single exit
  path (see Variations).

### State Authority and the Activity Pipeline (required)
All user-visible runtime state (sync phase, current activity, progress,
blockers, counters) has one authority and one write path.
- **SQLite is the source of truth.** Lifecycle and runtime state persists in
  the account's `sync_jobs.db`.
- **One writer.** Activity, phase and progress updates go through the
  central writer (`sync_engine/lifecycle_writer.rs`), which updates runtime
  and persistence together. A side write to the same state is a defect;
  `npm run check:sync-status-writes` guards the runtime status fields.
- **Structured activity contract:** `stage`, `progress_mode`, `current`,
  `total`, `unit`, `detail`, `updated_at`, `cycle_id`, never free-form
  strings.
- **The UI renders, it doesn't infer.** No UI-local state may compete with
  the persisted lifecycle/runtime state, and the UI never shows progress
  animation without active work.
- **One path to the window** (`docs/ui-data-flow.md`). SQLite triggers raise
  each account's change counters on every real change, and JSON writes say
  whether the window shows the file (`ShownInWindow`), so nothing changes
  what the window shows without the window hearing. Only the store in
  `src/state/` reads from the backend (`npm run check:ui-data-reads`);
  screens send actions and never reload or fetch.
- **Pause, resume, retry, startup restore and error transitions** are
  reflected immediately in the authoritative state and stay consistent
  across reloads.
- A state or status change is done only when one authoritative path is used
  end to end, the transitions above stay consistent in DB and UI, and the
  logs show each stage transition and its reason.

### Development-Phase Data Policy
SomeDrive is pre-1.0. Local config and state formats may change directly:
no migration code or legacy parsing paths, and testing from a clean
`~/.config/somedrive` is expected. Revisit before a stable release. The one
exception, approved for cross-app consistency: at start-up
`paths::migrate_legacy_settings_file` renames `app_settings.json` to
`settings.json` (if both exist it keeps `settings.json`, logs a warning and
leaves the old file untouched; if the rename fails, start-up stops with the
unreadable-settings error dialog).

### Frontend (`src/`)
- Strict TypeScript (no `any`), functional components, logic in hooks under
  `src/hooks/` (`useAppRuntime` composes the store with navigation and
  actions; `useUpdates`, `useToastManager`, `useWindowControls` own their
  concerns). Everything the window shows comes from the store in
  `src/state/`.
- `main.tsx` is entry wiring only; `App.tsx` composes shell, layout and hooks.
- `src/styles.css` only imports the focused files in `src/styles/`.
- **UI consistency contract:** shared layout primitives (`.page`,
  `.page-header`, cards, header actions) are defined once and reused; back
  control left, title centred, actions right; operational telemetry on
  operational pages, account settings pages configuration-only; replacing a
  pattern removes the old one in the same change.

### Commands
- `npm run tauri:dev` / `npm run tauri:build`: run or build the app.
- `npm run dev` / `npm run build` / `npm run preview`: the frontend alone
  (`build` typechecks first).
- `npm run typecheck`, `npm run lint`, `npm run check:structure`,
  `npm run check:sync-status-writes`, `npm run check:ui-data-reads`.
- `npm run icons:generate`: regenerate the app icons from `docs/logo.svg`.
- `npm run release:linux|prepare|notes|verify|promote`: see `docs/RELEASE.md`.
- In `src-tauri/`: `cargo fmt`, `cargo check`, `cargo clippy --all-targets`,
  `cargo test` (`cargo test -- <name>` for one test).

---

## Platform Notes

- **Linux only.** Wayland and X11 are both supported; the session
  environment is set before Tauri starts (App Standard section 2).
- **Packages first:** `.deb` and `.rpm` are the primary delivery, the
  AppImage the cross-distro fallback (`docs/install.sh`).
- **Tray:** a StatusNotifierItem. Stock GNOME needs the AppIndicator
  extension; without a tray, closing parks the window in the taskbar.
- **Sign-in** runs in an in-app webview window against Microsoft's login
  pages; tokens are stored by SomeDrive in `auth.json`, not in the webview.

---

## Variations

### From the App Standard
- **Window:** minimum 640 x 480. The corner resize handles in
  `AppShell.tsx` / `shell.css` / `useWindowControls.ts` are the standard's
  reference implementation.
- **Logging:** `debug/session.log` follows the standard (rotated to
  `last-session.log` at start-up, system info header, RFC 3339 timestamps,
  level in Settings -> Debug, panic hook). SomeDrive adds:
  - `debug/profiles/<account>.log`: each account's detailed sync trace,
    size-rotated (20 MB x 5). Only that account's warnings, errors and cycle
    milestones also reach the session log, which keeps it readable with
    several busy accounts.
  - `debug/raw/raw-<time>.log`: everything combined, only while the
    developer option "Raw logger mode" is on (last 10 kept).
  - Because SomeDrive runs for weeks, a session log over 20 MB rolls over to
    `session.log.1` mid-session.
- **Diagnostics:** the Logs block (Copy log, Open log, Open log folder,
  System info with copy, log level) is in Settings -> Debug, with raw logger
  mode. The Debug Tools page behind it holds UI Lab, the session log preview,
  per-account logs and the runtime snapshot.
- **Settings storage:** `migrate_legacy_fields` is not needed yet (no field
  has been renamed). An unreadable `settings.json` or `profiles.json`
  stops start-up; an unreadable or damaged `auth.json` means that account
  must sign in again.
- **Settings order:** General, Updates, Sync Performance, Debug.
- **CI:** Linux only, and it also runs `npm run check:sync-status-writes`
  and `npm run check:ui-data-reads`.

### From the Release and Update Standard
The step-by-step procedure is `docs/RELEASE.md`. Never run
`npm run release:promote` unless the maintainer asks.

- **Platforms:** Linux only. Three assets (`somedrive-X.Y.Z-linux-x64.deb`,
  `.rpm`, `.AppImage`), named only by `scripts/release-assets.mjs`. No
  Windows installer scripts and no Windows build or install-test job.
- **Install site:** `https://somedrive.org` (GitHub Pages from
  `main:/docs`).
- **Identity:** app ID `info.brumley.somedrive` (standard section 10): Tauri
  `identifier`, `APP_ID` (`src-tauri/src/app/identity.rs`; the GTK prgname
  and tray ID),
  and the launcher `src-tauri/packaging/info.brumley.somedrive.desktop`
  (Tauri's own `somedrive.desktop` is the Hidden stub from
  `bundler-stub.desktop.hbs`). `productName` is `somedrive`, so the binary,
  the deb/rpm package, the icon and the autostart entry
  (`~/.config/autostart/somedrive.desktop`) use that name; the window title
  and launcher show "SomeDrive". Earlier dev builds used `org.somedrive.app`
  and the `somedrive.desktop` / `SomeDrive.desktop` launcher and autostart
  names; `install.sh` and `uninstall.sh` clean those up. User data and
  sign-in tokens live in `~/.config/somedrive`; the webview data dir
  (`~/.local/share/<id>`) holds only Microsoft's login-page cookies, so the
  ID change needs no migration (at worst the sign-in page asks for the
  password again).
- **Pre-update hook and graceful exit:** SomeDrive never exits mid-transfer.
  Every exit drains sync first through one state machine,
  `src-tauri/src/app/sync_engine/sync_shutdown.rs`:
  1. *FinishingTransfers*: no new sync cycle, download or upload starts;
     transfers already running finish. Anything still running at the
     deadline is cancelled exactly as Pause does (downloads write to a temp
     file that is discarded, except that files of 64 MB or more keep their
     finished ranges to resume; the job is requeued).
  2. *StoppingWorkers*: every sync worker is cancelled and must fully exit,
     including its download worker tasks.
  3. *Stopped*: claimed/running jobs are returned to the queue. Everything
     else is already committed to the per-account SQLite DB, which is the
     sync authority.

  Budgets (`SyncShutdownReason`): **Update** 30 s + 20 s, **Exit** 10 s +
  10 s. The saved agent state of each account is not changed, so syncing
  accounts resume on the next start.
  - *Update:* `install_update` drains before starting the installer. If sync
    does not stop in time or the installer cannot be started, the update is
    abandoned: sync resumes and the update modal shows the error ("Finishing
    sync before updating…" is shown while it runs).
  - *Exit:* tray Quit, closing with close-to-tray off and SIGTERM/SIGINT/SIGHUP
    (`app::shutdown::exit_on_termination_signals`) all call `exit()`, and
    `RunEvent::Exit` runs `app::shutdown::finish_before_exit`. Only one
    shutdown ever runs: if one already started (an update), the exit path
    waits for it instead of draining twice. A session logout that kills the
    display without a signal cannot be intercepted; startup requeues any
    job left claimed.
  - *Installers:* `install.sh` and `uninstall.sh` give SomeDrive up to 25 s
    after SIGTERM ("Waiting for SomeDrive to finish syncing...") before
    SIGKILL, instead of the standard 1 s.
- **App shell:** tray menu Open SomeDrive, Pause all syncing, Resume all
  syncing, Quit (`src-tauri/src/app/tray/`, StatusNotifierItem only, as SomeDrive
  is Linux-only). The tray's pause/resume emits `accounts-updated` so the UI
  refreshes. "Start SomeDrive when I log in" registers
  `~/.config/autostart/somedrive.desktop` with `--start-hidden`, and an
  enabled entry is rewritten at startup so older entries gain the flag.
- **Build dependencies:** `libdbus-1-dev` (tao) and `xdg-utils` (bundled
  `xdg-open`) on top of the standard WebKitGTK set, in
  `src-tauri/packaging/linux/Containerfile`. No OpenSSL (reqwest uses
  rustls) and no tray library.
