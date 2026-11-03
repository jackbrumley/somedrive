use chrono::{Local, TimeZone};
use rusqlite::{params, Connection, OptionalExtension};

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
             PRAGMA synchronous = NORMAL;
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
                 remote_present INTEGER NOT NULL DEFAULT 0,
                 local_present INTEGER NOT NULL DEFAULT 0,
                 remote_size INTEGER NOT NULL DEFAULT 0,
                 local_size INTEGER NOT NULL DEFAULT 0,
                 remote_modified_ts INTEGER NOT NULL DEFAULT 0,
                 local_modified_ts INTEGER NOT NULL DEFAULT 0,
                 desired_action TEXT NOT NULL DEFAULT 'none',
                 conflict_state TEXT,
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
             CREATE TABLE IF NOT EXISTS sync_lifecycle_state (
                 profile_id TEXT PRIMARY KEY,
                 two_way_ready INTEGER NOT NULL DEFAULT 0,
                 bootstrap_scan_initialized INTEGER NOT NULL DEFAULT 0,
                 bootstrap_full_scan_completed INTEGER NOT NULL DEFAULT 0,
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
             CREATE TABLE IF NOT EXISTS sync_state_store (
                 profile_id TEXT PRIMARY KEY,
                 state_json TEXT NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_sync_throttle_events_lookup
                  ON sync_throttle_events(profile_id, direction, occurred_at)
             ;",
        )
        .map_err(|error| format!("Failed initializing sync jobs schema: {error}"))?;

    run_sync_jobs_migrations(&connection)?;

    Ok(connection)
}

fn run_sync_jobs_migrations(connection: &Connection) -> Result<(), String> {
    let current_version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("Failed reading sync_jobs schema version: {error}"))?;

    if current_version < 1 {
        add_sync_jobs_column_if_missing(
            connection,
            "run_state",
            "ALTER TABLE sync_jobs ADD COLUMN run_state TEXT NOT NULL DEFAULT 'idle'",
        )?;
        add_sync_jobs_column_if_missing(
            connection,
            "bytes_done",
            "ALTER TABLE sync_jobs ADD COLUMN bytes_done INTEGER NOT NULL DEFAULT 0",
        )?;
        add_sync_jobs_column_if_missing(
            connection,
            "bytes_total",
            "ALTER TABLE sync_jobs ADD COLUMN bytes_total INTEGER",
        )?;
        add_sync_jobs_column_if_missing(
            connection,
            "progress_updated_at",
            "ALTER TABLE sync_jobs ADD COLUMN progress_updated_at INTEGER",
        )?;
        connection
            .execute_batch(
                "UPDATE sync_jobs SET run_state = 'idle' WHERE run_state IS NULL;
                 UPDATE sync_jobs
                 SET run_state = 'idle'
                 WHERE state IN ('done', 'failed_terminal', 'retry_wait', 'queued', 'skipped');
                 UPDATE sync_jobs
                 SET run_state = 'claimed'
                 WHERE state = 'in_progress' AND run_state = 'idle';
                 UPDATE sync_jobs
                 SET progress_updated_at = COALESCE(progress_updated_at, updated_at)
                 WHERE progress_updated_at IS NULL;
                 PRAGMA user_version = 1;",
            )
            .map_err(|error| format!("Failed applying sync_jobs schema migration v1: {error}"))?;
    }

    let current_version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("Failed re-reading sync_jobs schema version: {error}"))?;
    if current_version < 2 {
        connection
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_sync_jobs_activity
                     ON sync_jobs(profile_id, direction, state, run_state, progress_updated_at);
                 PRAGMA user_version = 2;",
            )
            .map_err(|error| format!("Failed applying sync_jobs schema migration v2: {error}"))?;
    }

    let current_version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("Failed re-reading sync_jobs schema version: {error}"))?;
    if current_version < 3 {
        add_sync_files_column_if_missing(
            connection,
            "is_shared_reference",
            "ALTER TABLE sync_files ADD COLUMN is_shared_reference INTEGER NOT NULL DEFAULT 0",
        )?;
        add_sync_files_column_if_missing(
            connection,
            "shared_drive_id",
            "ALTER TABLE sync_files ADD COLUMN shared_drive_id TEXT",
        )?;
        add_sync_files_column_if_missing(
            connection,
            "shared_item_id",
            "ALTER TABLE sync_files ADD COLUMN shared_item_id TEXT",
        )?;
        add_sync_files_column_if_missing(
            connection,
            "shared_kind",
            "ALTER TABLE sync_files ADD COLUMN shared_kind TEXT",
        )?;
        connection
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_sync_files_remote_item
                     ON sync_files(profile_id, remote_item_id);
                 PRAGMA user_version = 3;",
            )
            .map_err(|error| format!("Failed applying sync_jobs schema migration v3: {error}"))?;
    }

    let current_version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("Failed re-reading sync_jobs schema version: {error}"))?;
    if current_version < 4 {
        add_sync_lifecycle_column_if_missing(
            connection,
            "activity_stage",
            "ALTER TABLE sync_lifecycle_state ADD COLUMN activity_stage TEXT NOT NULL DEFAULT 'idle'",
        )?;
        add_sync_lifecycle_column_if_missing(
            connection,
            "activity_progress_mode",
            "ALTER TABLE sync_lifecycle_state ADD COLUMN activity_progress_mode TEXT NOT NULL DEFAULT 'hidden'",
        )?;
        add_sync_lifecycle_column_if_missing(
            connection,
            "activity_current",
            "ALTER TABLE sync_lifecycle_state ADD COLUMN activity_current INTEGER",
        )?;
        add_sync_lifecycle_column_if_missing(
            connection,
            "activity_total",
            "ALTER TABLE sync_lifecycle_state ADD COLUMN activity_total INTEGER",
        )?;
        add_sync_lifecycle_column_if_missing(
            connection,
            "activity_unit",
            "ALTER TABLE sync_lifecycle_state ADD COLUMN activity_unit TEXT",
        )?;
        add_sync_lifecycle_column_if_missing(
            connection,
            "activity_detail",
            "ALTER TABLE sync_lifecycle_state ADD COLUMN activity_detail TEXT",
        )?;
        add_sync_lifecycle_column_if_missing(
            connection,
            "activity_cycle_id",
            "ALTER TABLE sync_lifecycle_state ADD COLUMN activity_cycle_id TEXT",
        )?;
        add_sync_lifecycle_column_if_missing(
            connection,
            "activity_updated_at",
            "ALTER TABLE sync_lifecycle_state ADD COLUMN activity_updated_at INTEGER NOT NULL DEFAULT 0",
        )?;
        connection
            .execute_batch("PRAGMA user_version = 4;")
            .map_err(|error| format!("Failed applying sync_jobs schema migration v4: {error}"))?;
    }

    let current_version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("Failed re-reading sync_jobs schema version: {error}"))?;
    if current_version < 5 {
        add_sync_lifecycle_column_if_missing(
            connection,
            "activity_cycle_id",
            "ALTER TABLE sync_lifecycle_state ADD COLUMN activity_cycle_id TEXT",
        )?;
        connection
            .execute_batch("PRAGMA user_version = 5;")
            .map_err(|error| format!("Failed applying sync_jobs schema migration v5: {error}"))?;
    }

    let current_version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("Failed re-reading sync_jobs schema version: {error}"))?;
    if current_version < 6 {
        add_sync_lifecycle_column_if_missing(
            connection,
            "large_delete_guard_approved",
            "ALTER TABLE sync_lifecycle_state ADD COLUMN large_delete_guard_approved INTEGER NOT NULL DEFAULT 0",
        )?;
        add_sync_lifecycle_column_if_missing(
            connection,
            "large_delete_pending_paths_json",
            "ALTER TABLE sync_lifecycle_state ADD COLUMN large_delete_pending_paths_json TEXT NOT NULL DEFAULT '[]'",
        )?;
        connection
            .execute_batch("PRAGMA user_version = 6;")
            .map_err(|error| format!("Failed applying sync_jobs schema migration v6: {error}"))?;
    }

    Ok(())
}

fn bool_to_sql(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

fn sql_to_bool(value: i64) -> bool {
    value != 0
}

fn add_sync_jobs_column_if_missing(
    connection: &Connection,
    column_name: &str,
    alter_sql: &str,
) -> Result<(), String> {
    let mut statement = connection
        .prepare("PRAGMA table_info(sync_jobs)")
        .map_err(|error| format!("Failed preparing sync_jobs schema query: {error}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("Failed querying sync_jobs schema info: {error}"))?;
    for row in rows {
        let existing = row.map_err(|error| format!("Failed reading sync_jobs schema row: {error}"))?;
        if existing == column_name {
            return Ok(());
        }
    }
    connection
        .execute(alter_sql, [])
        .map_err(|error| format!("Failed adding sync_jobs column '{column_name}': {error}"))?;
    Ok(())
}

fn add_sync_files_column_if_missing(
    connection: &Connection,
    column_name: &str,
    alter_sql: &str,
) -> Result<(), String> {
    let mut statement = connection
        .prepare("PRAGMA table_info(sync_files)")
        .map_err(|error| format!("Failed preparing sync_files schema query: {error}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("Failed querying sync_files schema info: {error}"))?;
    for row in rows {
        let existing = row.map_err(|error| format!("Failed reading sync_files schema row: {error}"))?;
        if existing == column_name {
            return Ok(());
        }
    }
    connection
        .execute(alter_sql, [])
        .map_err(|error| format!("Failed adding sync_files column '{column_name}': {error}"))?;
    Ok(())
}

fn add_sync_lifecycle_column_if_missing(
    connection: &Connection,
    column_name: &str,
    alter_sql: &str,
) -> Result<(), String> {
    let mut statement = connection
        .prepare("PRAGMA table_info(sync_lifecycle_state)")
        .map_err(|error| format!("Failed preparing sync_lifecycle_state schema query: {error}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("Failed querying sync_lifecycle_state schema info: {error}"))?;
    for row in rows {
        let existing = row
            .map_err(|error| format!("Failed reading sync_lifecycle_state schema row: {error}"))?;
        if existing == column_name {
            return Ok(());
        }
    }
    connection
        .execute(alter_sql, [])
        .map_err(|error| {
            format!(
                "Failed adding sync_lifecycle_state column '{column_name}': {error}"
            )
        })?;
    Ok(())
}

fn unix_seconds_to_rfc3339(value: i64) -> String {
    Local
        .timestamp_opt(value, 0)
        .single()
        .map(|instant| instant.to_rfc3339())
        .unwrap_or_else(|| Local::now().to_rfc3339())
}
