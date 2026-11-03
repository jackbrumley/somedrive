# SomeDrive Agent Manifesto & Guidelines

This document serves as a constitution for all agentic coding entities (and humans) operating within this repository. Integrity, cleanliness, and architectural soundness are our primary metrics of success.

---

## 🏛️ The Project Philosophy

### 1. Integrity Over Expediency
We do not value "quick hacks" that work today but create technical debt for tomorrow. If a feature or fix cannot be implemented cleanly, it should not be implemented until a proper architectural solution is found. 
- **No Shortcuts:** "Temporary" workarounds are forbidden. If a platform (like Wayland) restricts an action, we find the compliant API (like XDG Portals) instead of forcing a legacy hack.
- **No Half-Efforts:** Features must be substantially complete and polished. This includes proper error handling, logging, and UI feedback.
- **Clean Over Functional:** We would rather have a clean, well-organized codebase that is missing a feature than a messy one that has it.

### 2. Neatness, Tidiness, and OCD-Standard Code
Code is for humans to read, and only secondarily for machines to execute.
- **Semantic Clarity:** Variable names must be descriptive and intentional. Avoid abbreviations like `amt` for `amount` or `idx` for `index`.
- **Single Responsibility:** Functions and modules must do one thing and do it well. Large functions should be decomposed into logical units.
- **Formatting:** Strict adherence to `cargo fmt` and `npm run typecheck`.
- **Proactive Cleanup:** If you see messy code, redundant nesting, or illogical organization, you are expected to suggest a cleanup or fix it immediately (after confirming with the user).

### 3. Linux Display Server Support
Linux support targets both Wayland and X11, with clear platform boundaries.
- **Wayland Path:** Use **XDG Portals** (via `ashpd`) for hardware access (Microphone, Shortcuts, Input Emulation).
- **X11 Path:** Use native X11-compatible backends for shortcuts/input while keeping behavior aligned with Wayland as closely as possible.
- **Compositor Awareness:** Recognize that Wayland compositors (GNOME, KDE, Hyprland) have strict security models; keep those integrations explicit and future-proof.
- **Primary Delivery:** Prefer distro-native Linux packages (`.deb` / `.rpm`) where possible, and treat AppImage as the cross-distro fallback.

### 4. Root Cause First
We solve problems at their origin. If data is messy, redundant, or incorrect, do not "clean it up" at the consumer level (e.g., in the UI or intermediate wrappers). Trace the data back to its absolute source of truth and fix the generation/fetching logic there. A workaround is technical debt; a root-cause fix is engineering.

### 5. Lean, Durable Architecture (No Bloat)
We design for long-term maintainability as a solo-developed project. Architecture must remain clean and scalable without over-engineering.
- **Capability-Driven, Not Distro-Driven:** Organize by platform and protocol capabilities, not by distro names. Prefer runtime capability detection over hardcoded Fedora/GNOME/KDE branching.
- **One Owner Per Concern:** Session lifecycle, portal API integration, state transitions, and UI mapping should each have a clear single owner.
- **No Abstraction Without Payoff:** New modules or traits must reduce duplication, simplify reasoning, or improve reliability. Avoid "future-proof" layers that are unused.
- **Small, Localized Change Surface:** Future platform changes (portal updates, new compositor behavior) should require minor edits in capability/adapter modules, not architectural rewrites.
- **State Machines Over Ad-Hoc Flags:** For non-trivial flows (permissions, hotkeys, portal sessions), prefer explicit state transitions over scattered booleans.

### 6. Platform Adaptation Pattern
When implementing platform-sensitive features, follow this structure:
1. **Platform Boundary First:** Keep OS/display boundaries (`linux/wayland`, `linux/x11`, `windows`) as top-level separations.
2. **Provider Layer Second:** Within a platform, isolate backend/provider behavior (e.g., portal capabilities and session handling).
3. **Quirks Last:** Only add DE/provider-specific quirk modules when a real incompatibility is confirmed and cannot be solved generically.

This pattern keeps the codebase clean as new distros, compositor versions, or portal changes appear.

### 7. Development-Phase Data Policy (Clean Slate)
This project is currently in active development and testing. We optimize for speed of iteration and clarity, not backward compatibility.
- **No Backward Compatibility Requirement:** Local config/state formats can change directly when needed.
- **No Legacy Layers:** Do not add migration code, compatibility shims, fallback adapters, or legacy parsing paths.
- **Direct Schema Evolution:** If storage/schema changes, update the active model and related code paths directly.
- **Fresh-State Testing Expected:** During testing, it is valid and expected to remove local app data/accounts and re-run setup from a clean install.
- **Pre-Release Reassessment:** Compatibility policy can be revisited when moving toward beta/public release readiness.

### 8. State Authority & Activity Pipeline (Required)
All user-visible runtime state (sync phase, current activity, progress, blockers, counters) must have one authority and one write path.
- **Single Source of Truth:** SQLite-backed lifecycle/runtime state is authoritative by default.
- **One Writer API:** All activity/phase/progress updates must flow through one centralized backend writer that updates runtime + persistence together.
- **No Direct Side Writes:** Any code path that mutates the same state outside the central writer is a defect.
- **No Dual Authority:** UI-local inferred state must not compete with persisted lifecycle/runtime state.
- **Structured Activity Contract:** Represent activity as structured fields (`stage`, `progress_mode`, `current`, `total`, `unit`, `detail`, `updated_at`, `cycle_id`) instead of scattered free-form strings.
- **UI as Renderer:** Frontend renders authoritative activity state; it must not fabricate status from heuristics.
- **Pause/Resume Invariant:** Pause/resume/cancel transitions must be immediately reflected in authoritative state and stay consistent across hydration/reload.

### 9. Consistency, Single Responsibility, and Duplication Guardrails
- **One Owner Per Concern:** Lifecycle transitions, queue state, planner state, and UI mapping must each have a clear owner.
- **No Duplicate Decision Paths:** If the same business rule appears in multiple places, centralize it.
- **No Band-Aids:** Do not patch UI symptoms to hide backend state drift. Fix the source logic.
- **Naming Must Match Responsibility:** Module/class/function names must reflect actual responsibility. Rename stale legacy names.
- **No Silent Divergence:** If two paths can produce conflicting truth for the same data, treat it as a blocker, not a follow-up.

### 10. Proactive Integrity Review Duty (Mandatory)
Agents are expected to proactively detect and escalate architectural drift, even outside the immediate task.
- **Detect While Working:** Look for nearby violations (duplicate logic, split authority, stale/dead paths, inconsistent ownership, UI inference over source data).
- **Report Immediately:** If found, explicitly state what is inconsistent, why it is risky, and what root-cause fix is recommended.
- **Ask to Fold In:** If outside scope, ask whether to include cleanup in the same change.
- **Do Not Normalize Debt:** Never silently work around an inconsistency and move on.
- **Track Deferred Cleanup:** If deferred, include a concrete follow-up item in the response.

### 11. Definition of Done for State/Status Changes
A state/status task is incomplete unless all checks pass:
1. One authoritative state path is used end-to-end.
2. No direct writes bypass the central writer for the same state.
3. Pause, resume, retry, startup restore, and error transitions are consistent in DB and UI.
4. `cargo check` and `npm run typecheck` pass.
5. Logs clearly show stage transitions and reasons.
6. UI never shows active progress animation when there is no active work.

---

## 🛠️ Essential Commands

### Project-wide (Root)
Managed via **npm** scripts and the Tauri CLI.
- **Dependency Check:** `npm run deps:check`
  - Verifies required system dependencies and prints install commands when missing.
- **Dev:** `npm run tauri:dev`
  - Runs dependency checks and starts the Tauri development server.
- **Build:** `npm run tauri:build`
  - Runs dependency checks, builds the frontend, and packages the app.
- **Tauri CLI:** `npm run tauri -- <command>`
  - Use for tauri-specific tasks like `tauri icon` or `tauri info`.

### Backend (`src-tauri/`)
- **Lint:** `cargo clippy` (Static analysis) and `cargo fmt` (Formatting).
- **Check:** `cargo check` (Fast compilation check).
- **Test:** `cargo test` (Run all tests).
- **Single Test:** `cargo test -- <name>` (Execute a specific test function).
- **Doc:** `cargo doc --open` (Generate and view crate documentation).

### Frontend (`src/`)
- **Type Check:** `npm run typecheck`
  - Essential for verifying TypeScript integrity.
- **Lint:** `npm run lint`
  - Uses ESLint to enforce project styling rules.
- **Dev Server:** `npm run dev`
  - Starts the Vite dev server for UI-only iteration.
- **Preview:** `npm run preview`
  - Previews the production build of the UI.

---

## 🚀 Releases and Updates

Follow the
[Release and Update Standard](https://github.com/jackbrumley/jackbrumley.github.io/blob/main/RELEASE_STANDARD.md)
(shared with Voquill, Rusty G6 and Yambuck). The step-by-step procedure is
`docs/RELEASE.md`. Never run `npm run release:promote` unless the maintainer
asks.

SomeDrive's variations from the standard:
- **Platforms:** Linux only. Three assets (`somedrive-X.Y.Z-linux-x64.deb`,
  `.rpm`, `.AppImage`), named only by `scripts/release-assets.mjs`. No
  Windows installer scripts and no Windows build or install-test job.
- **Install site:** `https://brumley.info/somedrive` (GitHub Pages from
  `main:/docs`).
- **Identity:** app ID `info.brumley.somedrive` (standard section 10): Tauri
  `identifier`, `APP_ID` / GTK prgname (`src-tauri/src/platform/linux/wayland/env.rs`),
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
     file that is discarded; the job is requeued).
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

---

## 🏗️ Architecture & Patterns

### 1. Backend (Rust)
- **Async Flow:** Use `tokio` or `tauri::async_runtime` for all I/O, network, and audio operations. Never block the main thread.
- **Error Handling:** Use `anyhow` for internal propagation to maintain context.
- **Command Safety:** Return `Result<T, String>` for all `#[tauri::command]` functions. The error string is what the frontend `Promise.reject` receives.
- **State Management:** Use `AppState` (managed by Tauri) to hold shared resources like `Config`, `AudioStream`, or `RecordingState`.
- **Modularity:** Keep hardware-specific logic isolated in modules (e.g., `audio.rs`, `typing.rs`, `hotkey.rs`).

### 2. Frontend (Preact)
- **Strict TypeScript:** No `any`. Explicit interfaces for all data structures (API responses, State slices).
- **Hooks over Classes:** Use functional components and custom hooks (in `src/hooks/`) for logic isolation.
- **Styles:** Keep `src/styles.css` as an import aggregator only. Split style concerns into focused files under `src/styles/` (shell, pages, accounts, forms, sync, toast, etc.).
- **Component Styling:** Prefer local style ownership by concern; avoid adding new rules to a giant catch-all stylesheet.
- **Tauri Core:** Use `@tauri-apps/api` for communication with the backend.

#### UI Consistency Contract (Required)
- **Define Once, Reuse Everywhere:** Shared page layout primitives (`.page`, `.page-header`, `.page-subtitle`, card spacing, header action zones) are global contracts. Do not recreate them per page.
- **No Duplicate Patterns:** If multiple pages share a pattern, they must use the same component/classes. Avoid one-off spacing/alignment overrides unless there is a confirmed exception.
- **Header Contract:** Back control left, title centered to viewport, actions right. Keep one canonical implementation and avoid transform-based control centering that causes soft/blurry rendering.
- **Operational vs Settings Separation:** Runtime/synchronization telemetry belongs on operational pages; account settings pages must remain configuration-only.
- **Naming Must Match Reality:** Component/class names must reflect actual role (`Panel`, `Page`, `Section`). Do not keep legacy names like `Popover` when content is page-embedded.
- **Refactor Completion Rule:** When replacing a UI pattern, remove dead pages/components/styles in the same change (or immediately after) so the codebase has one source of truth.
- **Acceptance Check (UI Work):** Before marking complete, verify consistent spacing/header behavior across active pages and run `npm run typecheck` + `cargo check`.

### 3. Structure Contracts (Enforced)
- **File Size Ceiling:** No `.rs`, `.ts`, `.tsx`, or `.css` source file may exceed **1000 lines**.
- **Soft Size Target:** Most files should stay below 400-600 lines unless there is a clear, justified reason.
- **Semantic Splits Only:** Never split files into numeric shards (for example `part1`, `part2`, etc.). New files must be named by responsibility and grouped by domain concern.
- **Main Wiring Only (Rust):** `src-tauri/src/main.rs` is bootstrap-only and must contain no domain/business logic.
- **Main Wiring Only (Frontend Entry):** `src/main.tsx` is entry wiring only.
- **App Composition Only:** `src/App.tsx` composes shell/layout/hooks and should not hold feature/business logic.
- **Backend Composition Point:** Register Tauri commands in `src-tauri/src/lib.rs`; keep command implementations in `src-tauri/src/app/commands/` and domain logic in focused app modules.

---

## 📋 Platform Compatibility & Requirements

| Platform | Display Server | Audio Backend | Hardware Access |
| :--- | :--- | :--- | :--- |
| **Linux** | Wayland, X11 | ALSA / PulseAudio | Wayland: XDG Portals (`ashpd`), X11: native X11 backends |
| **Windows** | Desktop | WASAPI | CoreAudio API |

### Linux Permission Setup
On Wayland, the app should trigger standard XDG Portal prompts for microphone, global shortcuts, and remote desktop (input simulation). On X11, equivalent capabilities use native X11 backends and should still surface clear setup/readiness state in the UI.

---

## 🔄 Development Workflow for New Features

When adding a new feature, follow this sequence:
1.  **Analyze Environment:** Check for platform-specific constraints (Wayland and X11 where relevant).
2.  **Scaffold Backend:** Implement the logic in a new or existing Rust module.
3.  **Expose Command:** Create a `#[tauri::command]` in `src-tauri/src/app/commands/` and register it in `src-tauri/src/lib.rs`.
4.  **Implement UI:** Create the Preact component and hook it up to the command using `invoke`.
5.  **Verify Integrity:** Run `cargo clippy`, `npm run typecheck`, and `npm run lint`.
6.  **Test Platform Parity:** Verify the feature works on Linux (Wayland and X11) and Windows.

---

## 🚧 High-Priority Architectural Fixes (Current Debt)

Any agent working on this repo should prioritize the following cleanups:
1.  **Large Backend Module Split:** Break down `src-tauri/src/app/sync_engine.rs` into focused modules (worker, cycle, graph client, remote/local apply, runtime updates, state persistence).
2.  **Large Stylesheet Split:** Keep `src/styles.css` as an import-only entry and move rules into focused style files under `src/styles/`.
3.  **Runtime Hook Decomposition:** Keep `src/hooks/useAppRuntime.ts` as an orchestrator facade while moving concerns into focused hooks/modules.

---

## 🤖 Interaction Guidelines for Agents
- **Look for Improvement:** Don't just implement the request. Analyze the surrounding code for "mess" and offer to tidy it up.
- **Correct Inaccuracies Proactively:** If a user statement is technically incorrect or based on a false assumption, explicitly correct it and proceed with the correct approach. Do not silently follow an incorrect premise.
- **Ask, Don't Assume:** If a cleanup involves structural changes (like moving folders or renaming modules), always explain *why* it's cleaner and ask for approval.
- **Trace the Data:** Before proposing a fix for any data-related issue, trace the information back to its origin. Propose a fix for the source logic rather than a filter for the consumer.
- **Status Updates:** Use the centralized `emit_status_update` in Rust as the single source of truth for UI state. Avoid emitting ad-hoc events for standard states.
- **Platform Parity:** When adding a feature, ensure it is considered for Windows and Linux (Wayland and X11). If a platform requires specific logic, isolate it in a platform-specific module.
- **UI Consistency First:** Keep the UI behavior, structure, and interaction flow identical across systems whenever possible. Only diverge at the exact point where an OS/backend capability requires it (for example, system-managed shortcut configuration vs in-app configuration).
- **Documentation:** Proactively update `AGENTS.md` or other docs if you introduce a new architectural pattern or a major dependency.
- **Self-Verification:** Always run `cargo check` and `npm run typecheck` before declaring a task complete.
- **Git Commits:** Do not perform git commits without explicit user approval. Always ask for confirmation before running `git commit`.

### Solo-Scale Guardrails
- **Prefer Simplicity by Default:** Use the simplest clean solution that meets current requirements and known near-term needs.
- **Delay Splits Until Needed:** Do not create DE-specific files/folders until at least one concrete, recurring incompatibility exists.
- **Keep Files Focused:** A file should answer one question clearly. Split only when readability materially improves.
- **No Silent Failure Paths:** Always surface actionable errors in logs and, when relevant, to UI status.
- **Diagnostics Before Guesswork:** Add clear capability/version/runtime diagnostics before introducing conditional behavior.

### CodePerfect Mindset (Solo Operator Standard)
- **One Debug Lane Per Concern:** For each user-visible state (especially sync), there must be one authoritative source and one render path.
- **No Split Authority Debt:** If a UI decision reads from two competing sources, treat that as a defect and resolve it immediately.
- **Canonical Actions Required:** If UI shows a blocker (for example auth required), backend payload must include the concrete action set needed to recover.
- **Recovery Path Is Not Authority:** Snapshot/recovery fetches are transport safety only; ongoing truth must come from the authoritative pipeline.
- **Determinism Required:** For identical authoritative state and event sequence, backend payload and UI output must be identical; heuristic fallbacks that can produce alternate states are not allowed.

---

## ⚠️ Common Pitfalls to Avoid
- **Blocking the UI:** Never run expensive calculations or blocking I/O on the main thread.
- **Hardcoding Paths:** Always use the Tauri `PathResolver` or standard `dirs` crate to locate configuration and data directories.
- **Silent Failures:** Always log errors and, if relevant, notify the user via a Toast or Status update.
- **Inconsistent Naming:** Do not mix `camelCase` and `snake_case` in the same context. Follow the established patterns (Rust: `snake_case`, TS: `camelCase`).
- **Over-Engineering:** Prefer simple, readable code over complex "clever" solutions. If a function is hard to explain, it needs to be simplified.
- **Ignoring Warnings:** Treat compiler warnings as errors. Clean code means zero warnings.

---
*Clean code is a requirement, not a feature.*
