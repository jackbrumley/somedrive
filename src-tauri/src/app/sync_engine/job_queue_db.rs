use super::*;

/// Schema of the baseline engine (docs/sync-engine.md section 8). A database
/// written by an older engine is dropped and recreated, not migrated
/// (development-phase data policy).
pub(crate) const SYNC_SCHEMA_VERSION: i64 = 7;

/// Accounts whose schema is already in place in this process, so the DDL
/// runs once per account rather than on every connection.
pub(crate) static SYNC_SCHEMA_READY: std::sync::LazyLock<std::sync::Mutex<HashSet<String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashSet::new()));

pub(crate) const SYNC_SCHEMA_SQL: &str = "
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
        cloud_seen_at INTEGER NOT NULL DEFAULT 0,
        cloud_download_url TEXT,
        cloud_download_url_at INTEGER NOT NULL DEFAULT 0,
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
    -- The plan by action, covering the status counters: one index-only scan
    -- gives every count the UI shows (about 6 ms for 46,000 files, where
    -- three separate counts took about 140 ms). It replaces the narrower
    -- idx_sync_files_action, whose two columns it starts with.
    DROP INDEX IF EXISTS idx_sync_files_action;
    CREATE INDEX IF NOT EXISTS idx_sync_files_plan
        ON sync_files(profile_id, desired_action, cloud_present, local_present, is_shared_reference);
    CREATE INDEX IF NOT EXISTS idx_sync_files_remote_item
        ON sync_files(profile_id, remote_item_id);

    -- Cloud renames and moves waiting to be repeated on disk.
    CREATE TABLE IF NOT EXISTS sync_local_moves (
        profile_id TEXT NOT NULL,
        old_path TEXT NOT NULL,
        new_path TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        PRIMARY KEY(profile_id, old_path)
    );

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
        resync_started_at INTEGER,
        agent_state TEXT NOT NULL DEFAULT 'idle',
        last_sync_at TEXT,
        updated_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS sync_approved_deletes (
        profile_id TEXT NOT NULL,
        path TEXT NOT NULL,
        approved_at INTEGER NOT NULL,
        PRIMARY KEY(profile_id, path)
    );
    CREATE TABLE IF NOT EXISTS cleanup_hidden (
        profile_id TEXT NOT NULL,
        path TEXT NOT NULL,
        hidden_at INTEGER NOT NULL,
        PRIMARY KEY(profile_id, path)
    );
";

/// Tables of the pre-baseline engine, dropped when an old database is found.
pub(crate) const LEGACY_SYNC_TABLES: [&str; 9] = [
    "sync_approved_deletes",
    "cleanup_hidden",
    "sync_local_moves",
    "sync_jobs",
    "sync_files",
    "sync_issue_state",
    "sync_throttle_events",
    "sync_lifecycle_state",
    "sync_state_store",
];

pub(crate) fn sync_jobs_db_path(profile_id: &str) -> Result<PathBuf, String> {
    Ok(crate::paths::account_dir(profile_id)?.join("sync_jobs.db"))
}

/// Idle connections kept per account. Each worker or task that needs the
/// database takes one and returns it when done, instead of opening the file
/// (and setting its pragmas) for every query.
const CONNECTION_POOL_CAPACITY_PER_ACCOUNT: usize = 8;

static CONNECTION_POOL: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Vec<Connection>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// A connection on loan from the account's pool; it goes back on drop.
pub(crate) struct PooledConnection {
    profile_id: String,
    connection: Option<Connection>,
}

impl std::ops::Deref for PooledConnection {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.connection
            .as_ref()
            .expect("pooled connection present until drop")
    }
}

impl std::ops::DerefMut for PooledConnection {
    fn deref_mut(&mut self) -> &mut Connection {
        self.connection
            .as_mut()
            .expect("pooled connection present until drop")
    }
}

impl Drop for PooledConnection {
    fn drop(&mut self) {
        let Some(connection) = self.connection.take() else {
            return;
        };
        // A connection still inside a transaction is never reused.
        if !connection.is_autocommit() {
            return;
        }
        if let Ok(mut pool) = CONNECTION_POOL.lock() {
            let idle = pool.entry(self.profile_id.clone()).or_default();
            if idle.len() < CONNECTION_POOL_CAPACITY_PER_ACCOUNT {
                idle.push(connection);
            }
        }
    }
}

/// Idle pooled connections for `profile_id` (tests).
#[cfg(test)]
pub(crate) fn idle_pooled_connections(profile_id: &str) -> usize {
    CONNECTION_POOL
        .lock()
        .map(|pool| pool.get(profile_id).map_or(0, Vec::len))
        .unwrap_or(0)
}

pub(crate) fn open_sync_jobs_connection(profile_id: &str) -> Result<PooledConnection, String> {
    let pooled = CONNECTION_POOL
        .lock()
        .map_err(|_| "Sync database pool lock is poisoned".to_string())?
        .get_mut(profile_id)
        .and_then(Vec::pop);
    let connection = match pooled {
        Some(connection) => connection,
        None => open_new_sync_jobs_connection(profile_id)?,
    };
    Ok(PooledConnection {
        profile_id: profile_id.to_string(),
        connection: Some(connection),
    })
}

fn open_new_sync_jobs_connection(profile_id: &str) -> Result<Connection, String> {
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

/// Creates or rebuilds the account's sync schema, once per account per run.
///
/// No Rust lock is held while the database is touched or a line is logged:
/// logging an account line loads the account list, which opens this database
/// again, and a lock held here would deadlock start-up. An IMMEDIATE
/// transaction serialises the check-and-rebuild across connections instead.
pub(crate) fn ensure_sync_schema(profile_id: &str, connection: &Connection) -> Result<(), String> {
    let already_ready = SYNC_SCHEMA_READY
        .lock()
        .map_err(|_| "Sync schema lock is poisoned".to_string())?
        .contains(profile_id);
    if already_ready {
        return Ok(());
    }

    let transaction =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| format!("Failed starting sync schema transaction: {error}"))?;
    let version = transaction
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("Failed reading sync schema version: {error}"))?;
    let reset_from_version = (version != 0 && version != SYNC_SCHEMA_VERSION).then_some(version);
    if reset_from_version.is_some() {
        for table in LEGACY_SYNC_TABLES {
            transaction
                .execute_batch(&format!("DROP TABLE IF EXISTS {table};"))
                .map_err(|error| format!("Failed dropping legacy table {table}: {error}"))?;
        }
    }
    transaction
        .execute_batch(SYNC_SCHEMA_SQL)
        .map_err(|error| format!("Failed initializing sync schema: {error}"))?;
    transaction
        .execute_batch(&format!("PRAGMA user_version = {SYNC_SCHEMA_VERSION};"))
        .map_err(|error| format!("Failed writing sync schema version: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Failed committing sync schema: {error}"))?;

    SYNC_SCHEMA_READY
        .lock()
        .map_err(|_| "Sync schema lock is poisoned".to_string())?
        .insert(profile_id.to_string());
    if let Some(from_version) = reset_from_version {
        log::warn!(
            "{} SYNC_SCHEMA_RESET from_version={} to_version={} reason=engine_replaced",
            log_context::account_prefix(profile_id),
            from_version,
            SYNC_SCHEMA_VERSION
        );
    }
    Ok(())
}

pub(crate) fn bool_to_sql(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

pub(crate) fn sql_to_bool(value: i64) -> bool {
    value != 0
}

pub(crate) fn unix_seconds_to_rfc3339(value: i64) -> String {
    Local
        .timestamp_opt(value, 0)
        .single()
        .map(|instant| instant.to_rfc3339())
        .unwrap_or_else(|| Local::now().to_rfc3339())
}
