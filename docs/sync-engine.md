# Sync Engine: Baseline Reconciliation Design

Status: **accepted** (2026-11-04), not yet built. Replaces the two-phase "cloud first, then
two-way" model. When this is built it becomes the single description of how
SomeDrive decides what to do with a file.

---

## 1. Goal

SomeDrive behaves like the OneDrive desktop app, minus Files On-Demand:

- Add an account, it syncs. No further clicks, no modes, no switch to
  two-way. It runs unattended for weeks.
- Both sides converge on the same content. A change on either side reaches
  the other; a delete on either side reaches the other.
- **No file is ever lost.** Where SomeDrive cannot be sure, it keeps both
  copies. Anything it deletes locally goes to the desktop Trash. OneDrive's
  own recycle bin and version history cover the cloud side.

Files On-Demand (placeholders that download when opened) is out of scope;
nothing here blocks adding it later.

## 2. What is wrong today

Found by the data-safety review on 2026-11-04 (scenario IDs in brackets):

- The planner compares the two sides' **modified times** with each other.
  Downloads keep their download time, so after the first sync every local
  file looks newer and the whole drive is re-uploaded (A1). Cloud deletes of
  downloaded files are undone (A1). When both sides change, the newer time
  wins silently (A1).
- Downloads replace whatever is on disk at that path with no check: files
  already in the folder at first sync (A3) and local edits made since the
  last cycle (A2).
- The two-phase lifecycle (`two_way_ready`, the bootstrap gate, the
  "preparing baseline" step) exists to paper over the missing baseline, and
  its hand-over is where the damage happens.
- Finished jobs are never re-queued, so the first later change to a synced
  file stops every cycle with a materialization error (B1).
- The cloud item list and local snapshot live in one JSON blob
  (`PersistedSyncState`) saved every 25 results; a pause loses changes made
  since (B5), and the save gets slower as the drive grows.

## 3. The core idea: three states per item

For every item SomeDrive tracks three things:

| State | Source | Meaning |
|---|---|---|
| **Cloud** | Graph `/delta` | What OneDrive has now |
| **Local** | Local scan | What is on disk now |
| **Baseline** | Written by SomeDrive | What both sides held the last time they agreed |

Each side is compared **with the baseline, never with the other side**:

- *Cloud changed* = the cloud `cTag` (content tag) differs from the baseline
  `cTag`, or the item is gone.
- *Local changed* = the local size or modified time differs from the baseline
  local size and modified time, or the file is gone. (A content hash is only
  computed when size and time say "changed" but we need to know whether the
  content really differs, e.g. first-sync merges and conflicts.)

The baseline is written **only after an action succeeds**: after a download
lands (with the cloud `cTag` and the local size/time as written), after an
upload completes (with the `cTag` OneDrive returns and the local size/time
that was uploaded), or after a delete is confirmed on both sides (baseline
row removed).

This is how the OneDrive client, Dropbox, rclone bisync and Syncthing all
work; it is the piece SomeDrive is missing.

## 4. Decision table (files)

`=` same as baseline, `Δ` changed, `+` exists with no baseline, `∅` absent.

| # | Local | Cloud | Action |
|---|---|---|---|
| 1 | = | = | Nothing |
| 2 | = | Δ | Download (safe overwrite, §6.1) |
| 3 | Δ | = | Upload (`If-Match` the baseline eTag) |
| 4 | Δ | Δ | **Conflict**: keep both (§6.2) |
| 5 | ∅ | = | Delete in cloud (delete guard, §6.4) |
| 6 | = | ∅ | Delete locally, to Trash (delete guard) |
| 7 | ∅ | Δ | Download (cloud edit beats local delete; nothing lost) |
| 8 | Δ | ∅ | Upload as new (local edit beats cloud delete; nothing lost) |
| 9 | ∅ | ∅ | Remove baseline row |
| 10 | + | ∅ | Upload as new |
| 11 | ∅ | + | Download |
| 12 | + | + | Same size and hash → record baseline, no transfer. Different → conflict, keep both |

Row 12 is the **first-sync merge**: files already in the folder, or a
database rebuilt from scratch, are compared by content instead of being
overwritten. It also means the ~17,000 files from the current test run will
be recognised, not downloaded again.

Folders follow the same table with presence only (no content): a new folder
on either side is created on the other; a folder delete is carried across
only once everything inside it has been processed, and goes through the
delete guard like files.

## 5. One mode from the first sync

The two-phase model is removed: no `two_way_ready`, no bootstrap gate, no
"preparing two-way baseline" phase, no special first cycle. Every cycle:

1. **Cloud changes**: page through `/delta`; write each page's items into
   `sync_items` and the next-page link in **one transaction** (fixes B5).
2. **Local scan**: walk the sync root, write current local state into
   `sync_items` (skipping SomeDrive's own temp files, §6.6).
3. **Plan**: apply the decision table in SQL over `sync_items`; every row
   needing work gets a job **upserted to `queued` from any previous state**
   (fixes B1). Rows that now need nothing have their stale jobs removed.
4. **Execute**: downloads, uploads, folder creates, deletes and conflicts run
   through the existing job queue and workers.
5. **Commit baseline** per item as each action succeeds.

The first sync is simply a cycle where nothing has a baseline yet (rows 10,
11, 12). Progress in the UI is the same "Downloading your files" while it
mostly downloads.

## 6. Safety rules

### 6.1 Safe overwrite (downloads)
Before renaming a finished download into place, re-read the local file. If
it no longer matches the baseline (or the "absent" the plan assumed), do
**not** overwrite: keep the download as a conflict copy instead. Set the
file's modified time to the cloud time. Before the rename: `fsync` the temp
file, check its size (and `quickXorHash` when Graph supplies one); after:
`fsync` the directory (fixes A2, A8).

### 6.2 Conflicts: hold and ask
Never pick a winner, and never fill the folder with copies (the OneDrive
client's `-PCNAME` copies are how people end up with thousands of
duplicates). A file that changed on both sides (row 4), or is on both sides
with different content and no baseline (row 12), is **held**: nothing is
copied or overwritten, and that one path stops syncing while everything
else carries on. The same happens when a local file changes after a
download was planned: the download is discarded and the path held.

Held files are listed in the account's **Sync problems** tab with both
versions' size and time. The user chooses, for one file or many at once:

- **Keep this device's version:** it is uploaded; OneDrive's version stays
  in OneDrive's version history.
- **Keep OneDrive's version:** the local file goes to the Trash and
  OneDrive's version is downloaded.
- **Keep both:** only then is a copy made, `name (conflict YYYY-MM-DD HHMM
  on <hostname>).ext`, holding the local version; the original name takes
  OneDrive's version.

The choice is applied on the next cycle, and only if the local file is as it
was when the user chose; if it changed again, the choice is withdrawn and the
file stays held. The tab also lists names OneDrive cannot hold, OneNote
notebooks, malware-flagged files and downloads that failed for good, so
everything that needs the user is in one place.

### 6.3 Uploads
Uploads of an existing item send `If-Match: <baseline eTag>`. A 412 means the
cloud changed since the plan: re-plan that item (it becomes row 4). New
items use `conflictBehavior=fail`; a 409 means something appeared at that
path: re-plan. Folder creation uses `fail` and treats 409 as "exists, fetch
its id" (fixes A6, A9).

### 6.4 Deletes
- Local deletes go to the XDG Trash (`gio trash` semantics), never
  `remove_file`/`remove_dir_all` (fixes A4).
- One **delete guard** covers both directions: if a cycle plans more than
  the threshold (default 10, or 10% of tracked files, whichever is larger)
  deletes, it pauses deletes and asks. The approval re-checks each path
  against the **current** state before acting; anything that came back is
  dropped from the list (fixes A5). Small deletes spread over several cycles
  count against a rolling 24-hour window.
- A missing or empty sync root is never "everything deleted": the cycle
  stops with a "sync folder unavailable" issue (fixes the unmounted-drive
  case).

### 6.5 Names
- Two local names that differ only by case (`a.txt`, `A.txt`) cannot both
  exist in OneDrive: the second is reported as a per-file issue, not
  uploaded (fixes A7).
- Names OneDrive rejects (`: * ? " < > |`, trailing dot or space, reserved
  names) and non-UTF-8 names are per-file issues, shown in the UI, not cycle
  errors.

### 6.6 Temp and junk files
SomeDrive's `.somedrive-part-*.tmp` files are excluded from the local scan
and removed at cycle start (fixes C). An exclude list (Settings, later)
covers caches such as browser profiles.

## 7. Robustness

- **Jobs:** any action the plan needs is re-queued from any state (B1).
  Permanent per-file failures become per-file issues with a Retry, never a
  cycle-wide error.
- **Expired delta token (410 / `resyncRequired`):** clear the delta link and
  enumerate the drive again. Items not returned are **not** treated as
  deletes until a second enumeration confirms it (B2).
- **Paths come from item IDs:** each item stores its parent's item ID; paths
  are resolved from the parent chain, so a cloud folder rename moves its
  children correctly (B6).
- **Sign-in expiry mid-transfer:** in-flight jobs are cancelled and requeued
  (like pause), not failed (B4). Signing in again must be the same Microsoft
  account; a different account is rejected for an existing profile.
- **Transient cycle errors** (network, 5xx after retries): the worker retries
  the cycle with backoff instead of stopping until the user clicks Retry, and
  releases every claimed job on every error path (B3).
- **404 on download** re-checks the item through delta instead of marking it
  skipped forever (B7).

## 8. Data model

One table holds the per-item state; the JSON `PersistedSyncState` caches are
removed.

Implementation notes (2026-11-04):
- The table keeps the name `sync_files` (no rename churn); its columns are
  replaced by the set below (`cloud_*`, `local_*`, `base_*`).
- Schema version 7. A database from the old engine (`user_version` 1 to 6)
  is dropped and recreated, not migrated; the first sync then matches
  existing files by content (row 12). The schema is created once per
  account per process, not on every connection.
- Local times are stored in nanoseconds (`local_mtime_ns`).
- Cycle order: local scan first, then cloud delta pages (downloads stream
  while pages arrive, planned with the local state already known), then the
  full plan and execution.
- Cloud renames and moves, first version: the cloud columns move to the new
  path, so the old path plans "delete locally" (to Trash, guarded) and the
  new path plans "download". A true local rename is a later optimisation.
- Row 12 ("both new") plans a `verify` action that hashes the local file
  with QuickXorHash and compares it with the cloud hash.

`sync_files` columns:

| Column | Notes |
|---|---|
| `profile_id`, `path` | path is derived, kept for lookup and display |
| `remote_item_id`, `remote_parent_id` | identity in OneDrive |
| `is_dir` | |
| `cloud_present`, `cloud_size`, `cloud_mtime`, `cloud_etag`, `cloud_ctag`, `cloud_hash` | current cloud state |
| `local_present`, `local_size`, `local_mtime` | current local state |
| `base_present`, `base_size`, `base_local_mtime`, `base_etag`, `base_ctag`, `base_hash` | baseline |
| `desired_action`, `issue_code`, `issue_detail` | plan and per-file issue |

`sync_jobs` stays as the execution queue. `sync_lifecycle_state` loses
`two_way_ready` and the bootstrap fields.

Per the development-phase data policy there is no migration: the new engine
starts from an empty database. Existing local files are matched by row 12.

## 8a. Lessons from abraunegg/onedrive and rclone bisync

Researched 2026-11-04 (behaviours only; no code taken). These refine the
sections above and win where they differ:

- **Cloud changed** = the quickXorHash or size differs from the baseline.
  The cTag is only a fallback when either side has no hash; a missing value
  never counts as a change on its own. SharePoint rewrites Office/PDF files
  after upload, so the baseline after an upload records the hash OneDrive
  returns.
- **`If-Match`** only on creating an upload session; never on chunk uploads
  or deletes (both fail spuriously). Before a cloud delete, the item is read
  again and its hash compared with the baseline; a change cancels the delete
  (the planner then downloads it).
- **Content before action:** before an upload, a local file whose hash still
  equals the baseline hash only refreshes the baseline (a `touch` is not an
  edit). Before a conflict copy, matching local and cloud hashes adopt the
  file. A mass time change (clock or timezone shift) therefore only refreshes
  baselines and uploads nothing. If the sync folder is missing or empty while
  20 or more files are tracked (an unmounted drive, an empty mount point),
  the cycle stops with an issue instead of acting.
- **Folder deletes in the cloud** happen only when a fresh children listing
  shows the folder empty (Graph deletes recursively).
- **Conflict copies** (only when the user picks "Keep both") are made by
  copying the local file and checking the copy before the original is
  replaced.
- **Times:** after setting a downloaded file's time, the file is read back and
  that value is the baseline (OneDrive keeps whole seconds).
- **Excluded items** (per-file issues, never synced or deleted): OneNote
  notebooks (`package` facet) and everything inside them, local `.one` and
  `.onetoc2` files, items with a `malware` facet, shared shortcuts
  (`remoteItem`), and invalid local names: `" * : < > ? \ |`, leading or
  trailing spaces, a trailing dot, `CON PRN AUX NUL COM0-9 LPT0-9` (with any
  extension), `.lock`, `desktop.ini`, `~$*`, `*_vti_*`, control characters,
  paths of 400 characters or more, names not in Unicode NFC, and local names
  that differ only by case from another item.
- **Zero-byte files** have no hash: equal size 0 is a match. A non-empty file
  without a hash on either side is never assumed equal (conflict copy).
- **Cloud folder renames** still plan "delete old + download new" in the first
  version, so a rename of more than the guard threshold asks before acting.
  A true local rename by item ID is the next robustness step.

## 9. What is removed

- `two_way_ready`, `bootstrap_ready_for_two_way`,
  `reconcile_bootstrap_local_snapshot`, the "preparing two-way baseline"
  phase and its UI messages.
- The timestamp-comparison planner rules in `planner_transitions.rs`.
- `PersistedSyncState` (`remote_by_id`, `remote_path_to_id`,
  `local_snapshot`) and its periodic JSON save.
- The cycle-wide `enforce_planner_materialization_invariants` error (replaced
  by re-queueing and per-file issues).

## 10. Build order

Each step lands with tests and leaves the engine working.

1. **Test isolation.** `cargo test` stops writing into
   `~/.config/somedrive` (a test override in `paths.rs`); the stray test
   account folders are removed.
2. **Schema and delta fields.** `sync_items` with cloud, local and baseline
   columns; parse `eTag`, `cTag`, `parentReference.id` and
   `file.hashes.quickXorHash` from delta; write delta pages and the
   next-page link in one transaction; drop the JSON caches.
3. **Planner.** The decision table (§4) as SQL over `sync_items`, unit
   tested row by row; jobs upserted to `queued` from any state.
4. **Safe execution.** Safe overwrite, mtime set on download, fsync and size
   or hash check, `If-Match` uploads, `fail` conflict behaviour, conflict
   copies, baseline commit after each success.
5. **Deletes.** Trash for local deletes; the two-direction delete guard with
   re-check on approval; the missing-root stop.
6. **Remove the two-phase lifecycle** and its UI wording.
7. **Robustness.** 410 resync, sign-in expiry as requeue, automatic cycle
   retry with backoff, 404 re-check, name and case issues, temp file
   cleanup.

Steps 2 to 6 have to land together before the engine is used on real data;
step 7 can follow.

## 11. Testing before real data

Run on a **test Microsoft account** with an empty sync folder first, then a
folder that already holds some of the same files. Scenarios, each checked on
both sides afterwards:

- First sync: cloud-only files download; local-only files upload; identical
  files on both sides are matched without transfer; different files at the
  same path produce a conflict copy.
- Edit locally → uploaded; edit in OneDrive web → downloaded.
- Edit both before a cycle → conflict copy, both versions present.
- Delete locally → deleted in cloud (recycle bin); delete in cloud → local
  file in Trash.
- Delete locally while edited in cloud → cloud version restored locally.
- Rename a folder in OneDrive with files inside → local tree moves, nothing
  duplicated.
- Delete 20 files at once → guard asks; restore 5 before approving → only 15
  deleted.
- Unmount or rename the sync folder → sync stops with an issue, nothing
  deleted in cloud.
- Pause, quit and kill (`kill -9`) mid-download and mid-upload → no partial
  files, no lost changes, nothing re-uploaded.
- Sign-in expires mid-sync → downloads requeue; signing in again resumes.
- Case collision and an invalid name → per-file issues, sync continues.

Only after these pass does SomeDrive run on the real account.

## 12. Decisions

Agreed 2026-11-04:

- Conflict copies are named `name (conflict YYYY-MM-DD HHMM on <hostname>).ext`.
- The delete guard asks before more than 10 files or 10% of tracked files,
  whichever is larger, in either direction.
- Local deletes always go to the Trash.
- This document replaces the earlier sync plan documents; their lasting
  rules (database authority, single status writer) are in AGENTS.md.
