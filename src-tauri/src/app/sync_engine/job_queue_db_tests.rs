//! Tests for the sync database itself: the connection pool, schema rebuilds
//! and lifecycle rows.
use super::*;

#[test]
fn pooled_connections_are_reused_but_never_inside_a_transaction() {
    let profile_id = test_profile_id("connection-pool");
    {
        let connection = open_sync_jobs_connection(&profile_id).expect("open");
        connection
            .execute_batch("BEGIN; INSERT INTO sync_throttle_events (profile_id, direction, occurred_at) VALUES ('x', 'y', 1);")
            .expect("open a transaction and leave it");
    }
    let idle = idle_pooled_connections(&profile_id);
    assert_eq!(
        idle, 0,
        "a connection left in a transaction is dropped, not pooled"
    );

    {
        let _connection = open_sync_jobs_connection(&profile_id).expect("open");
    }
    let idle = idle_pooled_connections(&profile_id);
    assert_eq!(idle, 1, "a finished connection goes back to the pool");
    let reused = open_sync_jobs_connection(&profile_id).expect("reuse");
    assert!(reused.is_autocommit());
    let events: i64 = reused
        .query_row(
            "SELECT COUNT(1) FROM sync_throttle_events WHERE profile_id = 'x'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(events, 0, "the abandoned transaction was rolled back");
}

#[test]
fn a_database_from_an_older_engine_is_rebuilt_on_first_open() {
    let profile_id = test_profile_id("schema-reset");
    let db_path = sync_jobs_db_path(&profile_id).expect("db path");
    std::fs::create_dir_all(db_path.parent().expect("parent")).expect("db folder");
    {
        let old = Connection::open(&db_path).expect("create old db");
        old.execute_batch(
            "CREATE TABLE sync_jobs (id INTEGER PRIMARY KEY, legacy_only TEXT);
             PRAGMA user_version = 1;",
        )
        .expect("write old schema");
    }

    let connection = open_sync_jobs_connection(&profile_id).expect("open rebuilds");
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, SYNC_SCHEMA_VERSION);
    assert!(
        connection.is_autocommit(),
        "the rebuild transaction is closed"
    );
    let legacy_columns: i64 = connection
        .query_row(
            "SELECT COUNT(1) FROM pragma_table_info('sync_jobs') WHERE name = 'legacy_only'",
            [],
            |row| row.get(0),
        )
        .expect("columns");
    assert_eq!(legacy_columns, 0, "the old table was replaced");
    // A second connection opened while the first is held must not block.
    let _second = open_sync_jobs_connection(&profile_id).expect("second open");
}

#[test]
fn a_missing_lifecycle_row_is_recreated_from_the_saved_profile_only_once() {
    let profile_id = test_profile_id("lifecycle-seed");
    assert!(read_sync_lifecycle_row(&profile_id)
        .expect("read")
        .is_none());

    let created = ensure_sync_lifecycle_row(&profile_id, "paused", Some("2026-11-04T10:00:00Z"))
        .expect("seed");
    assert!(created);
    let row = read_sync_lifecycle_row(&profile_id)
        .expect("read")
        .expect("row");
    assert_eq!(row.agent_state, "paused");
    assert_eq!(row.last_sync_at.as_deref(), Some("2026-11-04T10:00:00Z"));
    assert_eq!(row.activity_progress_mode, "hidden");

    persist_sync_lifecycle_agent_state(&profile_id, "syncing").expect("change state");
    let created_again = ensure_sync_lifecycle_row(&profile_id, "paused", None).expect("seed again");
    assert!(!created_again, "an existing row is never overwritten");
    let row = read_sync_lifecycle_row(&profile_id)
        .expect("read")
        .expect("row");
    assert_eq!(row.agent_state, "syncing");
}

/// A deferred transaction that reads and then writes fails at once with
/// "database is locked" when another connection wrote in between (SQLite
/// cannot upgrade a stale read snapshot, and busy_timeout does not apply).
/// Every engine write transaction must take the write lock up front.
#[test]
fn engine_write_transactions_take_the_write_lock_up_front() {
    let engine_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app/sync_engine");
    let mut offenders = Vec::new();
    for entry in std::fs::read_dir(&engine_dir).expect("engine dir") {
        let path = entry.expect("entry").path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();
        if !name.ends_with(".rs") || name.ends_with("_tests.rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("read source");
        if source.contains(concat!(".transaction", "()")) {
            offenders.push(name);
        }
    }
    assert!(
        offenders.is_empty(),
        "use transaction_with_behavior(TransactionBehavior::Immediate) in: {offenders:?}"
    );
}

#[test]
fn planner_counters_come_from_one_scan_and_add_up() {
    let profile_id = test_profile_id("planner-counters");
    let connection = open_sync_jobs_connection(&profile_id).expect("open");
    for (path, action, cloud, local, shared) in [
        ("a", "download", 1, 0, 0),
        ("b", "download", 1, 0, 0),
        ("c", "upload", 0, 1, 0),
        ("d", "none", 1, 1, 0),
        ("e", "none", 1, 1, 1),
    ] {
        connection
            .execute(
                "INSERT INTO sync_files (profile_id, path, desired_action, cloud_present,
                    local_present, is_shared_reference, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
                params![profile_id, path, action, cloud, local, shared],
            )
            .expect("row");
    }
    drop(connection);

    let counters = read_sync_file_planner_counters(&profile_id).expect("counters");
    assert_eq!(counters.cloud_discovered_total, 4);
    assert_eq!(counters.local_discovered_total, 3);
    assert_eq!(counters.shared_reference_total, 1);
    assert_eq!(counters.need_download_total, 2);
    assert_eq!(counters.need_upload_total, 1);
    assert_eq!(counters.none_total, 2);
}
