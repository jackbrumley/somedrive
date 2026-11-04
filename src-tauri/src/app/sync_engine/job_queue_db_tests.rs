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
