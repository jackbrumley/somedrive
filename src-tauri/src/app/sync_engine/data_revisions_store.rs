use super::*;

// Change counters for what the window shows (docs/ui-data-flow.md). SQLite
// triggers raise them on every real change, so no code path can change
// what the window shows without the window hearing about it: the status
// emitter watches the counters and sends a status update when one moves,
// and the window reloads only the area whose counter moved.
//
// Only real changes count (`OLD.x IS NOT NEW.x`): a scan that rewrites the
// same values, and download progress, raise nothing.

/// Created with the schema. The triggers are dropped and recreated on each
/// start, so a changed definition always takes effect.
pub(crate) const DATA_REVISIONS_SQL: &str = "
    CREATE TABLE IF NOT EXISTS ui_data_revisions (
        area TEXT PRIMARY KEY,
        revision INTEGER NOT NULL
    );
    INSERT OR IGNORE INTO ui_data_revisions (area, revision)
        VALUES ('activity', 0), ('problems', 0), ('cleanup', 0);

    -- activity: transfers and deletes, running and finished.
    DROP TRIGGER IF EXISTS ui_activity_job_insert;
    CREATE TRIGGER ui_activity_job_insert AFTER INSERT ON sync_jobs
    BEGIN
        UPDATE ui_data_revisions SET revision = revision + 1 WHERE area = 'activity';
    END;
    DROP TRIGGER IF EXISTS ui_activity_job_update;
    CREATE TRIGGER ui_activity_job_update AFTER UPDATE OF state, run_state, finished_at ON sync_jobs
    WHEN OLD.state IS NOT NEW.state
      OR OLD.run_state IS NOT NEW.run_state
      OR OLD.finished_at IS NOT NEW.finished_at
    BEGIN
        UPDATE ui_data_revisions SET revision = revision + 1 WHERE area = 'activity';
    END;
    DROP TRIGGER IF EXISTS ui_activity_job_delete;
    CREATE TRIGGER ui_activity_job_delete AFTER DELETE ON sync_jobs
    BEGIN
        UPDATE ui_data_revisions SET revision = revision + 1 WHERE area = 'activity';
    END;

    -- problems: Sync problems and Not synced (a file's problem, conflict
    -- choice or shared-item flag, and downloads that failed for good).
    DROP TRIGGER IF EXISTS ui_problems_file_insert;
    CREATE TRIGGER ui_problems_file_insert AFTER INSERT ON sync_files
    WHEN NEW.issue_code IS NOT NULL OR NEW.is_shared_reference = 1
    BEGIN
        UPDATE ui_data_revisions SET revision = revision + 1 WHERE area = 'problems';
    END;
    DROP TRIGGER IF EXISTS ui_problems_file_update;
    CREATE TRIGGER ui_problems_file_update AFTER UPDATE ON sync_files
    WHEN OLD.issue_code IS NOT NEW.issue_code
      OR OLD.issue_detail IS NOT NEW.issue_detail
      OR OLD.resolution IS NOT NEW.resolution
      OR OLD.is_shared_reference IS NOT NEW.is_shared_reference
      OR OLD.path IS NOT NEW.path
      OR ((NEW.issue_code IS NOT NULL OR NEW.is_shared_reference = 1)
          AND (OLD.local_size IS NOT NEW.local_size
            OR OLD.local_mtime_ns IS NOT NEW.local_mtime_ns
            OR OLD.cloud_size IS NOT NEW.cloud_size
            OR OLD.cloud_mtime IS NOT NEW.cloud_mtime))
    BEGIN
        UPDATE ui_data_revisions SET revision = revision + 1 WHERE area = 'problems';
    END;
    DROP TRIGGER IF EXISTS ui_problems_file_delete;
    CREATE TRIGGER ui_problems_file_delete AFTER DELETE ON sync_files
    WHEN OLD.issue_code IS NOT NULL OR OLD.is_shared_reference = 1
    BEGIN
        UPDATE ui_data_revisions SET revision = revision + 1 WHERE area = 'problems';
    END;
    DROP TRIGGER IF EXISTS ui_problems_job_update;
    CREATE TRIGGER ui_problems_job_update AFTER UPDATE OF state ON sync_jobs
    WHEN OLD.state IS NOT NEW.state
      AND (OLD.state = 'failed_terminal' OR NEW.state = 'failed_terminal')
    BEGIN
        UPDATE ui_data_revisions SET revision = revision + 1 WHERE area = 'problems';
    END;
    DROP TRIGGER IF EXISTS ui_problems_job_delete;
    CREATE TRIGGER ui_problems_job_delete AFTER DELETE ON sync_jobs
    WHEN OLD.state = 'failed_terminal'
    BEGIN
        UPDATE ui_data_revisions SET revision = revision + 1 WHERE area = 'problems';
    END;
    -- The deletes held for review by the large-delete guard.
    DROP TRIGGER IF EXISTS ui_problems_delete_review_insert;
    CREATE TRIGGER ui_problems_delete_review_insert AFTER INSERT ON sync_lifecycle_state
    WHEN NEW.large_delete_pending_paths_json IS NOT '[]'
    BEGIN
        UPDATE ui_data_revisions SET revision = revision + 1 WHERE area = 'problems';
    END;
    DROP TRIGGER IF EXISTS ui_problems_delete_review;
    CREATE TRIGGER ui_problems_delete_review
    AFTER UPDATE OF large_delete_pending_paths_json ON sync_lifecycle_state
    WHEN OLD.large_delete_pending_paths_json IS NOT NEW.large_delete_pending_paths_json
    BEGIN
        UPDATE ui_data_revisions SET revision = revision + 1 WHERE area = 'problems';
    END;

    -- cleanup: the files the Clean up tab compares, and what was hidden.
    DROP TRIGGER IF EXISTS ui_cleanup_file_insert;
    CREATE TRIGGER ui_cleanup_file_insert AFTER INSERT ON sync_files
    WHEN NEW.is_dir = 0
    BEGIN
        UPDATE ui_data_revisions SET revision = revision + 1 WHERE area = 'cleanup';
    END;
    DROP TRIGGER IF EXISTS ui_cleanup_file_update;
    CREATE TRIGGER ui_cleanup_file_update AFTER UPDATE ON sync_files
    WHEN NEW.is_dir = 0 AND (
         OLD.path IS NOT NEW.path
      OR OLD.is_shared_reference IS NOT NEW.is_shared_reference
      OR OLD.cloud_present IS NOT NEW.cloud_present
      OR OLD.local_present IS NOT NEW.local_present
      OR OLD.base_present IS NOT NEW.base_present
      OR OLD.cloud_size IS NOT NEW.cloud_size
      OR OLD.cloud_mtime IS NOT NEW.cloud_mtime
      OR OLD.cloud_hash IS NOT NEW.cloud_hash
      OR OLD.base_hash IS NOT NEW.base_hash
      OR OLD.base_cloud_size IS NOT NEW.base_cloud_size
      OR OLD.local_size IS NOT NEW.local_size
      OR OLD.base_size IS NOT NEW.base_size
      OR OLD.local_mtime_ns IS NOT NEW.local_mtime_ns
      OR OLD.base_local_mtime_ns IS NOT NEW.base_local_mtime_ns
      OR OLD.desired_action IS NOT NEW.desired_action
      OR OLD.issue_code IS NOT NEW.issue_code)
    BEGIN
        UPDATE ui_data_revisions SET revision = revision + 1 WHERE area = 'cleanup';
    END;
    DROP TRIGGER IF EXISTS ui_cleanup_file_delete;
    CREATE TRIGGER ui_cleanup_file_delete AFTER DELETE ON sync_files
    WHEN OLD.is_dir = 0
    BEGIN
        UPDATE ui_data_revisions SET revision = revision + 1 WHERE area = 'cleanup';
    END;
    DROP TRIGGER IF EXISTS ui_cleanup_hidden_insert;
    CREATE TRIGGER ui_cleanup_hidden_insert AFTER INSERT ON cleanup_hidden
    BEGIN
        UPDATE ui_data_revisions SET revision = revision + 1 WHERE area = 'cleanup';
    END;
    DROP TRIGGER IF EXISTS ui_cleanup_hidden_delete;
    CREATE TRIGGER ui_cleanup_hidden_delete AFTER DELETE ON cleanup_hidden
    BEGIN
        UPDATE ui_data_revisions SET revision = revision + 1 WHERE area = 'cleanup';
    END;
";

/// The account's change counters.
pub(crate) fn read_data_revisions(
    connection: &Connection,
) -> Result<sync_runtime::DataRevisions, String> {
    let mut revisions = sync_runtime::DataRevisions::default();
    let mut statement = connection
        .prepare_cached("SELECT area, revision FROM ui_data_revisions")
        .map_err(|error| format!("Failed preparing change counters query: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| format!("Failed reading change counters: {error}"))?;
    for row in rows {
        let (area, revision) =
            row.map_err(|error| format!("Failed reading a change counter: {error}"))?;
        let revision = revision.max(0) as u64;
        match area.as_str() {
            "activity" => revisions.activity = revision,
            "problems" => revisions.problems = revision,
            "cleanup" => revisions.cleanup = revision,
            _ => {}
        }
    }
    Ok(revisions)
}

/// The account's change counters, read on their own (the status emitter).
pub fn read_account_data_revisions(
    profile_id: &str,
) -> Result<sync_runtime::DataRevisions, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    read_data_revisions(&connection)
}

#[cfg(test)]
mod data_revisions_tests {
    use super::*;

    fn revisions(profile_id: &str) -> sync_runtime::DataRevisions {
        read_account_data_revisions(profile_id).expect("revisions")
    }

    fn insert_file(profile_id: &str, path: &str, issue_code: Option<&str>) {
        let connection = open_sync_jobs_connection(profile_id).expect("open");
        connection
            .execute(
                "INSERT INTO sync_files (profile_id, path, cloud_present, local_present,
                     cloud_size, cloud_hash, issue_code, updated_at)
                 VALUES (?1, ?2, 1, 1, 10, 'hash', ?3, 0)",
                params![profile_id, path, issue_code],
            )
            .expect("insert");
    }

    fn execute(profile_id: &str, sql: &str) {
        let connection = open_sync_jobs_connection(profile_id).expect("open");
        connection.execute(sql, []).expect("execute");
    }

    #[test]
    fn a_new_file_moves_cleanup_and_a_problem_moves_problems() {
        let profile_id = test_profile_id("revisions-insert");
        let before = revisions(&profile_id);
        insert_file(&profile_id, "a.txt", None);
        let after = revisions(&profile_id);
        assert_eq!(after.cleanup, before.cleanup + 1);
        assert_eq!(after.problems, before.problems, "no problem, no change");

        insert_file(&profile_id, "b.txt", Some("name_invalid"));
        assert_eq!(revisions(&profile_id).problems, before.problems + 1);
    }

    #[test]
    fn rewriting_the_same_values_moves_nothing() {
        let profile_id = test_profile_id("revisions-same");
        insert_file(&profile_id, "a.txt", Some("name_invalid"));
        let before = revisions(&profile_id);
        execute(
            &profile_id,
            "UPDATE sync_files SET cloud_size = 10, cloud_hash = 'hash', issue_code = 'name_invalid',
                 updated_at = 99",
        );
        assert_eq!(
            revisions(&profile_id),
            before,
            "a scan's rewrite is not a change"
        );

        execute(&profile_id, "UPDATE sync_files SET cloud_hash = 'other'");
        let after = revisions(&profile_id);
        assert_eq!(after.cleanup, before.cleanup + 1);
    }

    #[test]
    fn a_finished_delete_moves_activity_and_hiding_moves_cleanup() {
        let profile_id = test_profile_id("revisions-activity");
        let before = revisions(&profile_id);
        record_finished_delete(&profile_id, HISTORY_DELETED_REMOTELY, "a.txt", 10).expect("record");
        assert_eq!(revisions(&profile_id).activity, before.activity + 1);

        execute(
            &profile_id,
            &format!(
                "INSERT INTO cleanup_hidden (profile_id, path, hidden_at) VALUES ('{profile_id}', 'x', 0)"
            ),
        );
        assert_eq!(revisions(&profile_id).cleanup, before.cleanup + 1);
    }

    #[test]
    fn deletes_held_for_review_move_problems() {
        let profile_id = test_profile_id("revisions-guard");
        let before = revisions(&profile_id);
        persist_large_delete_guard_state(
            &profile_id,
            &LargeDeleteGuardState {
                pending_paths: vec!["a.txt".to_string()],
                ..LargeDeleteGuardState::default()
            },
        )
        .expect("hold");
        assert!(revisions(&profile_id).problems > before.problems);
    }

    #[test]
    fn a_download_failing_for_good_moves_problems() {
        let profile_id = test_profile_id("revisions-failed");
        execute(
            &profile_id,
            &format!(
                "INSERT INTO sync_jobs (profile_id, direction, item_id, path, state, created_at, updated_at)
                 VALUES ('{profile_id}', 'download', 'id1', 'a.txt', 'in_progress', 0, 0)"
            ),
        );
        let before = revisions(&profile_id);
        execute(
            &profile_id,
            "UPDATE sync_jobs SET bytes_done = 5, progress_updated_at = 1",
        );
        assert_eq!(revisions(&profile_id), before, "progress is not a change");

        execute(
            &profile_id,
            "UPDATE sync_jobs SET state = 'failed_terminal'",
        );
        let after = revisions(&profile_id);
        assert_eq!(after.problems, before.problems + 1);
        assert_eq!(after.activity, before.activity + 1);
    }
}
