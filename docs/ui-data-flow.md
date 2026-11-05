# How the window stays current

Everything the window shows has **one path** from where it is stored to the
screen. Wiring it wrong is meant to be impossible rather than caught in
review, so both ends are enforced mechanically:

1. **Nothing changes in storage without the window hearing about it.** The
   storage layer itself announces every change.
2. **Nothing reaches the screen except through the store.** Only
   `src/state/` reads from the backend, and a build check fails otherwise.

The sync engine's own rules (one writer for runtime state, SQLite as the
source of truth) are in `AGENTS.md` under *State Authority* and in
`docs/sync-engine.md`.

---

## 1. Storage announces every change

### Each account's database: change counters

`sync_jobs.db` holds a `ui_data_revisions` table with one counter per area
of the window. SQLite triggers (`sync_engine/data_revisions_store.rs`)
raise a counter whenever a row it covers changes:

| Area | Shown in | Raised when |
| :--- | :--- | :--- |
| `activity` | Activity tab | A row in `sync_jobs` is added, removed, or changes state or finish time |
| `problems` | Sync problems, Not synced, the large-delete review | A file's problem, conflict choice or shared-item flag changes; a download fails for good or stops being failed; the deletes held for review change |
| `cleanup` | Clean up | A file is added or removed, or its presence, size, hash, time or plan changes; `cleanup_hidden` changes |

Only real changes count: every trigger compares old and new values
(`OLD.x IS NOT NEW.x`). The scan every 15 seconds rewrites the same values
and raises nothing; download progress raises nothing either (it travels on
the live status, below).

Because the triggers live in the database, a new code path that writes these
tables is covered without doing anything. The table and triggers are created
with the schema (`CREATE ... IF NOT EXISTS`, triggers dropped and recreated
on each start), so adding or changing a trigger needs no schema version
bump and no resync.

### JSON files: a required flag

`json_store::write_json` takes a `ShownInWindow` argument, so every save must
say whether the window shows that file. Profiles, settings and `auth.json`
say `Yes` and raise an in-memory app data revision; the activity log, which
is written every cycle and not shown, says `No`.

### Sending it to the window

The status emitter thread (`sync_runtime/status_events.rs`) runs a
`ChangeWatcher` (`sync_runtime/change_watcher.rs`):

- every **500 ms** it reads each account's counters (one small query per
  account). When they moved, it sends that account's status as a
  `sync-status` event. The status always carries the counters
  (`dataRevisions`), read in the same database snapshot as the rest of it;
- every **100 ms** it compares the app data revision and sends
  `app-data-changed` when it moved.

Code may still ask for an immediate status update (transfers do, for
progress); the watcher is what guarantees nothing is missed when it doesn't.

## 2. Only the store reads

`src/state/` is the store:

| File | Holds |
| :--- | :--- |
| `backendReads.ts` | Every backend read the window makes |
| `useAppData.ts` | Live account status (`sync-status`, numbered; a gap rereads the snapshot), the account list and settings (reread on `app-data-changed`), system info |
| `accountAreas.ts` | Per-account lists: Sync problems and Not synced, Clean up (only while its tab is open), the large-delete review (only while deletes are held). Each is reread when its counter moves |
| `useRevisionedRead.ts`, `coalesce.ts` | One read at a time: a change during a read runs it once more afterwards |

Screens read from the store and send actions (delete, pause, save a setting).
An action never reloads anything; its effect comes back through the counters.
The UI Lab supplies sample lists through `AccountAreasContext` instead of
the backend.

Auto-start is the one setting the backend does not save (the desktop holds
it), so the store rereads settings itself after changing it.

### The build check

`npm run check:ui-data-reads` (`scripts/check-ui-data-reads.mjs`, run in CI)
fails if any file outside `src/state/`:

- invokes a read command (`get_*`, `list_*`, `*_snapshot`);
- invokes a command named in a variable (it cannot be checked);
- reads auto-start's state;
- listens to `sync-status` or `app-data-changed`.

## Adding something the window shows

- **New data in an account's database:** add triggers for its table and
  columns to the area it belongs to (or a new area: a counter row, a field
  in `DataRevisions`, and a store hook), with a test that a real change
  raises the counter and a rewrite of the same values does not.
- **New JSON file:** pass `ShownInWindow::Yes` if the window shows it.
- **New screen data:** add the read to `backendReads.ts` and expose it from
  the store. The build check stops it being read anywhere else.
