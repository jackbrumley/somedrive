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

Settings → Sync → "When a file changes on both sides" can be switched from
**Ask me** (the default) to **Keep the newest**: a held file then gets the
newer side's version chosen automatically, applied through the same checks
as a user's choice; the older version stays in OneDrive's version history or
the Trash.

### 6.3 Uploads
Uploads of an existing item send `If-Match: <baseline eTag>`. A 412 means the
cloud changed since the plan: re-plan that item (it becomes row 4). New
items use `conflictBehavior=fail`; a 409 means something appeared at that
path: re-plan. Folder creation uses `fail` and treats 409 as "exists, fetch
its id" (fixes A6, A9).

### 6.4 Deletes
- Local deletes go to the XDG Trash (`gio trash` semantics), never
  `remove_file`/`remove_dir_all` (fixes A4).
- The **delete guard** covers deletes made on this computer: if a cycle
  would delete more than the threshold (default 100 files, or
  `SOMEDRIVE_SYNC_LARGE_DELETE_THRESHOLD`) from OneDrive, counting this
  computer's deletes of the last 5 minutes, it holds them and asks. The
  window adds up a folder a file manager removes in pieces; a folder counts
  as every file in it. The warning names the earlier deletes when they
  pushed the batch over. The approval re-checks each path against the
  **current** state before acting; anything that came back is dropped from
  the list (fixes A5).
- Deletes made in the cloud are never held and do not count: OneDrive asked
  when they were made, its recycle bin keeps them, and the local copy goes
  to the Trash.
- A missing or empty sync root is never "everything deleted": the cycle
  stops with a "sync folder unavailable" issue (fixes the unmounted-drive
  case).
- Deletes the user confirmed inside SomeDrive (the Clean up tab, after
  typing "delete") are recorded in `sync_approved_deletes`. The guard lets
  them through without asking again and does not count them towards its
  limit; other deletes in the same cycle are still held as usual. An
  approval lasts a day and ends as soon as its path is no longer tracked.
- A folder deleted here whose every cloud item is a planned delete goes to
  OneDrive with **one** request instead of a check and a delete per file
  (which would hit Microsoft's request limits for a big folder). The folder
  is read again first: it must still have the eTag the cloud changes last
  reported and exactly the expected bytes, and the delete itself is
  conditional on that eTag (`If-Match`). Anything that does not line up,
  or any untracked, skipped or excluded item inside, falls back to per-item
  deletes (`remote_deletes.rs`).
- Per-item file deletes run many at a time in the transfer pool
  (`transfer_pool.rs`) under the same adaptive limit as downloads; folders
  follow one at a time, deepest first, as each must be empty by then.

### 6.4a Clean up
The account page's Clean up tab suggests identical copies and lists large
files, worked out from the sync database alone (`cleanup_finder.rs`): a
copy has the same name and the same OneDrive hash and size as a file that
stays. Suggestions go as high up the folder tree as possible (a whole
folder; else the identical part of a folder that is at least 90% copies;
else the copies in one folder), and a greedy pass guarantees one copy of
every file stays even if every suggestion is deleted. Folders named "old",
"copy" or "duplicates" are suggested first, copies filed by date (a year
folder or a folder directly inside one, such as `Archive/2019/02`) last, so
the dated copy is the one kept; then the older of two snapshots goes first.
Hidden paths are left out of matching altogether. A delete works the
suggestion out again, compares every file byte for byte with the copy that
stays, refuses if a folder holds anything else on disk, then moves it to
the Trash and approves the deletes for the guard (`cleanup_delete.rs`).

Identical copies and Large files are separate lists, one on screen at a
time. Copies are confirmed by typing "delete". A large file can be deleted
there only when it is the only copy, confirmed by typing its file name; a
large file with a copy points to Identical copies, and the backend refuses
to delete it from Large files.

### 6.5 Names
- Two local names that differ only by case (`a.txt`, `A.txt`) cannot both
  exist in OneDrive: the second is reported as a per-file issue, not
  uploaded (fixes A7).
- Names OneDrive rejects (`: * ? " < > |`, trailing dot or space, reserved
  names) and non-UTF-8 names are per-file issues, shown in the UI, not cycle
  errors.

### 6.6 Temp files and exclude rules
SomeDrive's `.somedrive-part-*.tmp` files are excluded from the local scan
and removed when found (fixes C).

Exclude rules (Settings → Excluded from sync) keep paths out of sync in both
directions. A rule with a `/` is a path from the top of the sync folder (it
and everything under it, e.g. `Documents/Chromium`); a rule without one
matches a name anywhere (`node_modules`, `*.tmp`). The local scan does not
enter excluded folders, and excluded rows are marked before every plan
(also after each delta page, so nothing excluded starts downloading). An
excluded item is never uploaded, downloaded or deleted: a synced file that
disappears from the scan because of a new rule is not deleted in OneDrive.
Removing the rule brings it back under normal sync.

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

## 7a. Throughput

Downloads are the bulk of a first sync, so the download path avoids repeated
setup work:

- **Shared HTTP clients:** built once per cycle, so connections and TLS
  sessions are reused instead of opened per file. Graph API requests use
  HTTP/2 (small requests multiplexed on one connection). File contents use a
  separate HTTP/1.1 client with one connection per transfer: over HTTP/2 all
  parallel downloads shared one connection's flow-control window, which held
  32 downloads to about 2 MB/s combined on a 57 MB/s line.
- **Direct download links:** when the change feed includes
  `@microsoft.graph.downloadUrl`, it is stored with the item and used while
  it is under 50 minutes old, skipping one Graph request per file. A link
  refused with 401/403/404/410 falls back to the normal Graph download.
  Microsoft did not include these links in the change feed for the personal
  account tested (2026-11-04), so there every download goes through Graph.
- **Connection pool:** each account reuses its SQLite connections; a
  connection returned inside an open transaction is dropped, never reused.
- **Folder flush per batch:** a downloaded file's folder is flushed once per
  batch of results, before that batch is recorded as synced, instead of
  after every file.
- **Adaptive rate, aiming for a steady equilibrium:** two limits per
  account, parallel downloads (start 8, floor 4, ceiling the "Parallel
  downloads" setting) and download starts per second (start 8, floor 2,
  ceiling 40). Microsoft limits files per minute as well as bandwidth, so
  the start rate is what keeps thousands of tiny files under its quota.
  - Up: at most one small step per 10 seconds while downloads succeed
    (+1 parallel, +10% starts per second), so the rate creeps up rather
    than jumping.
  - Down: on a 429 both drop to three quarters. A 429 only counts when its
    request started after the last cut; answers to requests sent at the old
    rate are ignored, so one burst of 429s is one cut, not a cascade.
  - After a "wait N seconds" answer, one request goes first as a probe;
    every other request holds 2 more seconds. If the probe is refused, a new
    wait starts and nothing else was sent; otherwise the held requests are
    released over about 3 seconds.
  - Logged as `DOWNLOAD_LIMIT_RAISED` / `DOWNLOAD_LIMIT_LOWERED` with both
    values.
  - Transfers have no total deadline; they fail only when no data arrives
    for 60 seconds.
  - After a 429 the limits climb back one step a minute, only to 80% of
    the level Microsoft refused, then probe above it one small step a
    minute, so the rate settles instead of repeatedly paying a 30-second
    stop.
  - **One limit for the account.** Microsoft limits the account as a
    whole, so uploads and cloud deletes run in the transfer pool
    (`transfer_pool.rs`) under the same limits, feed it their successes
    and 429s, and wait out the same throttle window. Big uploads (8 MB or
    more) keep to the large-file share of the slots.
- **Uploads:** many files at once in the transfer pool. A big file goes
  through an upload session in 31.25 MiB pieces (100 × 320 KiB, the
  multiple OneDrive asks for), which must be sent in order. The session is
  saved (`sync_upload_sessions`), so an upload cut off by a pause, restart
  or dropped connection continues from the offset OneDrive reports, as
  long as the file's size and time are unchanged and the session has not
  expired (`upload_session.rs`).
- **Finding local changes:** the sync folder is watched (inotify, through
  the `notify` crate; `local_watch.rs`). A cycle scans this device only
  when the watcher saw a change, on the first cycle, when the exclude
  rules or the folder changed, when the watcher is unavailable or failed,
  or every 10 minutes as a safety net; otherwise it logs
  `LOCAL_SCAN_SKIPPED` and plans from the database, which the engine keeps
  current with its own transfers and deletes. A cycle starts every 15
  seconds for OneDrive's changes, or as soon as local changes have been
  quiet for 2 seconds. Reads and SomeDrive's own partial downloads are not
  changes; dropped events (queue overflow) are. A watcher that could not
  start (no folder, the system's watch limit) is retried with the
  10-minute scan, every cycle scans meanwhile, and the account page says
  so (`local_changes_watched`).
- **Content check on every core:** rows present on both sides are hashed
  on one worker thread per CPU core (`content_check.rs`), and the window
  shows "Checking files against OneDrive · N of M files". Each file is read
  again after hashing; one that changed meanwhile is skipped and planned
  again next cycle, never recorded as matching.
- **Big files in ranges, resumable:** Microsoft's file server limits each
  connection to a couple of MB/s, so files of 64 MB or more download in
  16 MB ranges, 8 at a time, into a pre-sized partial file
  (`.somedrive-partial-<key>.part`, the key including the cloud version).
  Each finished range is flushed and recorded in `.somedrive-partial-<key>.ranges`;
  a stall, a 429, an expired link or a restart fetches only the missing
  ranges. The whole file is checked against OneDrive's hash before it is put
  in place (a mismatch discards the partial). The local scan never syncs
  these files and removes ones untouched for 7 days. A server that ignores
  ranges falls back to a single download.
- **Size lanes:** files of 8 MB or more may hold a quarter of the slots (at
  least one) while smaller files are waiting; slots no small file needs go
  to large files.

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
- Cloud renames and moves are repeated locally as renames (section 8a).
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
- **`If-Match`** on creating an upload session; never on chunk uploads
  (they fail spuriously). A cloud file delete is first sent with `If-Match`
  on the eTag last seen, which the planner only deletes when it is the
  synced version. OneDrive sometimes refuses that without a content change,
  so a refusal is never taken as a change: the item is then read and its
  hash compared with the baseline, and a real change cancels the delete
  (the planner then downloads it). A file without an eTag goes straight to
  that read.
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
  notebooks (`package` facet) and everything inside them (loose `.one` and
  `.onetoc2` files outside a notebook sync as ordinary files), items with a
  `malware` facet, shared shortcuts
  (`remoteItem`), and invalid local names: `" * : < > ? \ |`, leading or
  trailing spaces, a trailing dot, `CON PRN AUX NUL COM0-9 LPT0-9` (with any
  extension), `.lock`, `desktop.ini`, `~$*`, `*_vti_*`, control characters,
  paths of 400 characters or more, names not in Unicode NFC, and local names
  that differ only by case from another item.
- **Zero-byte files** have no hash: equal size 0 is a match. A non-empty file
  without a hash on either side is never assumed equal (conflict copy).
- **Cloud renames and moves** are repeated on this device as a rename: the
  item's whole record (cloud, local and baseline) moves to the new path and
  the rename is queued in `sync_local_moves`, then carried out right after
  the delta page is stored (and at the start of the next cycle if the app
  stopped in between). Only when the new path is already taken does it fall
  back to "download the new path, remove the old copy through the delete
  guard".

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
  whichever is larger, in either direction. Replaced 2026-11-06: more than
  100 files in 5 minutes, deletes made on this computer only (section 6.4).
- Local deletes always go to the Trash.
- This document replaces the earlier sync plan documents; their lasting
  rules (database authority, single status writer) are in AGENTS.md.
