use chrono::{Local, TimeZone};
use rusqlite::{params, Connection, OptionalExtension};

/// Schema of the baseline engine (docs/sync-engine.md section 8). A database
/// written by an older engine is dropped and recreated, not migrated
/// (development-phase data policy).
const SYNC_SCHEMA_VERSION: i64 = 7;

/// Accounts whose schema is already in place in this process, so the DDL
/// runs once per account rather than on every connection.
static SYNC_SCHEMA_READY: std::sync::LazyLock<std::sync::Mutex<HashSet<String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashSet::new()));

const SYNC_SCHEMA_SQL: &str = "
    CREATE TABLE IF NOT EXISTS sync_jobs (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        profile_id TEXT NOT NULL,
        direction TEXT NOT NULL,
        item_id TEXT NOT NULL,
        path TEXT NOT NULL,
        remote_size INTEGER NOT NULL DEFAULT 0,
        remote_modified_ts INTEGER NOT NULL DEFAULT 0,
        state TEXT NOT NULL,
        run_state TEXT NOT NULL DEFAULT 'idle',
        attempt_count INTEGER NOT NULL DEFAULT 0,
        last_error TEXT,
        next_retry_at INTEGER,
        lease_owner TEXT,
        lease_until INTEGER,
        bytes_done INTEGER NOT NULL DEFAULT 0,
        bytes_total INTEGER,
        progress_updated_at INTEGER,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        started_at INTEGER,
        finished_at INTEGER,
        UNIQUE(profile_id, direction, item_id)
    );
    CREATE INDEX IF NOT EXISTS idx_sync_jobs_scheduler
        ON sync_jobs(profile_id, direction, state, next_retry_at);
    CREATE INDEX IF NOT EXISTS idx_sync_jobs_leases
        ON sync_jobs(profile_id, direction, lease_until);
    CREATE INDEX IF NOT EXISTS idx_sync_jobs_activity
        ON sync_jobs(profile_id, direction, state, run_state, progress_updated_at);

    -- One row per path: the current cloud state, the current local state and
    -- the baseline (what both sides held when they last agreed).
    CREATE TABLE IF NOT EXISTS sync_files (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        profile_id TEXT NOT NULL,
        path TEXT NOT NULL,
        is_dir INTEGER NOT NULL DEFAULT 0,
        is_shared_reference INTEGER NOT NULL DEFAULT 0,
        shared_drive_id TEXT,
        shared_item_id TEXT,
        shared_kind TEXT,
        remote_item_id TEXT,
        remote_parent_id TEXT,
        cloud_present INTEGER NOT NULL DEFAULT 0,
        cloud_size INTEGER NOT NULL DEFAULT 0,
        cloud_mtime INTEGER NOT NULL DEFAULT 0,
        cloud_etag TEXT,
        cloud_ctag TEXT,
        cloud_hash TEXT,
        local_present INTEGER NOT NULL DEFAULT 0,
        local_size INTEGER NOT NULL DEFAULT 0,
        local_mtime_ns INTEGER NOT NULL DEFAULT 0,
        base_present INTEGER NOT NULL DEFAULT 0,
        base_size INTEGER NOT NULL DEFAULT 0,
        base_cloud_size INTEGER NOT NULL DEFAULT 0,
        base_local_mtime_ns INTEGER NOT NULL DEFAULT 0,
        base_etag TEXT,
        base_ctag TEXT,
        base_hash TEXT,
        desired_action TEXT NOT NULL DEFAULT 'none',
        issue_code TEXT,
        issue_detail TEXT,
        resolution TEXT,
        resolution_local_mtime_ns INTEGER,
        updated_at INTEGER NOT NULL,
        UNIQUE(profile_id, path)
    );
    CREATE INDEX IF NOT EXISTS idx_sync_files_action
        ON sync_files(profile_id, desired_action);
    CREATE INDEX IF NOT EXISTS idx_sync_files_remote_item
        ON sync_files(profile_id, remote_item_id);

    CREATE TABLE IF NOT EXISTS sync_issue_state (
        profile_id TEXT PRIMARY KEY,
        issue_code TEXT NOT NULL,
        issue_message TEXT NOT NULL,
        issue_actions_json TEXT NOT NULL,
        issue_path TEXT,
        issue_secondary_path TEXT,
        updated_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS sync_throttle_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        profile_id TEXT NOT NULL,
        direction TEXT NOT NULL,
        occurred_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_sync_throttle_events_lookup
        ON sync_throttle_events(profile_id, direction, occurred_at);
    CREATE TABLE IF NOT EXISTS sync_lifecycle_state (
        profile_id TEXT PRIMARY KEY,
        delta_link TEXT,
        active_delta_next_link TEXT,
        last_cycle_at TEXT,
        phase TEXT NOT NULL DEFAULT 'idle',
        phase_message TEXT NOT NULL DEFAULT 'Idle',
        remote_scan_complete INTEGER NOT NULL DEFAULT 0,
        activity_stage TEXT NOT NULL DEFAULT 'idle',
        activity_progress_mode TEXT NOT NULL DEFAULT 'hidden',
        activity_current INTEGER,
        activity_total INTEGER,
        activity_unit TEXT,
        activity_detail TEXT,
        activity_cycle_id TEXT,
        activity_updated_at INTEGER NOT NULL DEFAULT 0,
        large_delete_guard_approved INTEGER NOT NULL DEFAULT 0,
        large_delete_pending_paths_json TEXT NOT NULL DEFAULT '[]',
        agent_state TEXT NOT NULL DEFAULT 'idle',
        last_sync_at TEXT,
        updated_at INTEGER NOT NULL
    );
";

/// Tables of the pre-baseline engine, dropped when an old database is found.
const LEGACY_SYNC_TABLES: [&str; 6] = [
    "sync_jobs",
    "sync_files",
    "sync_issue_state",
    "sync_throttle_events",
    "sync_lifecycle_state",
    "sync_state_store",
];

fn sync_jobs_db_path(profile_id: &str) -> Result<PathBuf, String> {
    Ok(crate::paths::account_dir(profile_id)?.join("sync_jobs.db"))
}

fn open_sync_jobs_connection(profile_id: &str) -> Result<Connection, String> {
    let db_path = sync_jobs_db_path(profile_id)?;
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed creating sync jobs directory '{}': {}",
                parent.display(),
                error
            )
        })?;
    }

    let connection = Connection::open(&db_path).map_err(|error| {
        format!(
            "Failed opening sync jobs DB '{}': {}",
            db_path.display(),
            error
        )
    })?;
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA synchronous = NORMAL;",
        )
        .map_err(|error| format!("Failed configuring sync jobs DB: {error}"))?;
    ensure_sync_schema(profile_id, &connection)?;
    Ok(connection)
}

fn ensure_sync_schema(profile_id: &str, connection: &Connection) -> Result<(), String> {
    let mut ready = SYNC_SCHEMA_READY
        .lock()
        .map_err(|_| "Sync schema lock is poisoned".to_string())?;
    if ready.contains(profile_id) {
        return Ok(());
    }

    let version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("Failed reading sync schema version: {error}"))?;
    if version != 0 && version != SYNC_SCHEMA_VERSION {
        log::warn!(
            "{} SYNC_SCHEMA_RESET from_version={} to_version={} reason=engine_replaced",
            log_context::account_prefix(profile_id),
            version,
            SYNC_SCHEMA_VERSION
        );
        for table in LEGACY_SYNC_TABLES {
            connection
                .execute_batch(&format!("DROP TABLE IF EXISTS {table};"))
                .map_err(|error| format!("Failed dropping legacy table {table}: {error}"))?;
        }
    }

    connection
        .execute_batch(SYNC_SCHEMA_SQL)
        .map_err(|error| format!("Failed initializing sync schema: {error}"))?;
    connection
        .execute_batch(&format!("PRAGMA user_version = {SYNC_SCHEMA_VERSION};"))
        .map_err(|error| format!("Failed writing sync schema version: {error}"))?;
    ready.insert(profile_id.to_string());
    Ok(())
}

fn bool_to_sql(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

fn sql_to_bool(value: i64) -> bool {
    value != 0
}

fn unix_seconds_to_rfc3339(value: i64) -> String {
    Local
        .timestamp_opt(value, 0)
        .single()
        .map(|instant| instant.to_rfc3339())
        .unwrap_or_else(|| Local::now().to_rfc3339())
}
