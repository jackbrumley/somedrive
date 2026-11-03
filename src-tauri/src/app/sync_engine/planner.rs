#[cfg(test)]
mod planner_tests {
    use super::*;

    fn test_profile_id(label: &str) -> String {
        format!(
            "planner-test-{label}-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        )
    }

    fn clear_profile_rows(profile_id: &str) {
        let connection = open_sync_jobs_connection(profile_id).expect("open sync jobs db");
        connection
            .execute("DELETE FROM sync_files WHERE profile_id = ?1", params![profile_id])
            .expect("clear sync_files");
        connection
            .execute("DELETE FROM sync_jobs WHERE profile_id = ?1", params![profile_id])
            .expect("clear sync_jobs");
    }

    // a test fixture that writes one sync_files row column by column.
    #[allow(clippy::too_many_arguments)]
    fn insert_sync_file_row(
        profile_id: &str,
        path: &str,
        remote_present: bool,
        local_present: bool,
        remote_modified_ts: i64,
        local_modified_ts: i64,
        remote_size: u64,
        local_size: u64,
        remote_item_id: Option<&str>,
    ) {
        let connection = open_sync_jobs_connection(profile_id).expect("open sync jobs db");
        connection
            .execute(
                "INSERT INTO sync_files (
                    profile_id, path, is_dir, is_shared_reference, shared_drive_id, shared_item_id, shared_kind,
                    remote_item_id, remote_present, local_present,
                    remote_size, local_size, remote_modified_ts, local_modified_ts,
                    desired_action, conflict_state, updated_at
                ) VALUES (
                    ?1, ?2, 0, 0, NULL, NULL, NULL,
                    ?3, ?4, ?5,
                    ?6, ?7, ?8, ?9,
                    'none', NULL, ?10
                )",
                params![
                    profile_id,
                    path,
                    remote_item_id,
                    bool_to_sql(remote_present),
                    bool_to_sql(local_present),
                    remote_size as i64,
                    local_size as i64,
                    remote_modified_ts,
                    local_modified_ts,
                    current_unix_seconds(),
                ],
            )
            .expect("insert sync_files row");
    }

    fn set_shared_reference(profile_id: &str, path: &str) {
        let connection = open_sync_jobs_connection(profile_id).expect("open sync jobs db");
        connection
            .execute(
                "UPDATE sync_files
                 SET is_shared_reference = 1
                 WHERE profile_id = ?1 AND path = ?2",
                params![profile_id, path],
            )
            .expect("mark shared reference");
    }

    #[test]
    fn planner_derives_download_upload_and_conflict_actions() {
        let profile_id = test_profile_id("actions");
        clear_profile_rows(&profile_id);

        insert_sync_file_row(
            &profile_id,
            "remote-only.txt",
            true,
            false,
            100,
            0,
            40,
            0,
            Some("remote-only-id"),
        );
        insert_sync_file_row(
            &profile_id,
            "local-only.txt",
            false,
            true,
            0,
            120,
            0,
            50,
            None,
        );
        insert_sync_file_row(
            &profile_id,
            "remote-newer.txt",
            true,
            true,
            200,
            100,
            10,
            10,
            Some("remote-newer-id"),
        );
        insert_sync_file_row(
            &profile_id,
            "local-newer.txt",
            true,
            true,
            100,
            200,
            10,
            10,
            Some("local-newer-id"),
        );
        insert_sync_file_row(
            &profile_id,
            "size-conflict.txt",
            true,
            true,
            300,
            300,
            10,
            20,
            Some("size-conflict-id"),
        );

        let counters = recompute_sync_file_actions(&profile_id, true).expect("recompute planner actions");
        assert_eq!(counters.need_download_total, 2);
        assert_eq!(counters.need_upload_total, 2);
        assert_eq!(counters.conflict_total, 1);

        let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
        let action_for_path = |path: &str| -> String {
            connection
                .query_row(
                    "SELECT desired_action FROM sync_files WHERE profile_id = ?1 AND path = ?2",
                    params![&profile_id, path],
                    |row| row.get::<_, String>(0),
                )
                .expect("read desired action")
        };

        assert_eq!(action_for_path("remote-only.txt"), "download");
        assert_eq!(action_for_path("local-only.txt"), "upload");
        assert_eq!(action_for_path("remote-newer.txt"), "download");
        assert_eq!(action_for_path("local-newer.txt"), "upload");
        assert_eq!(action_for_path("size-conflict.txt"), "conflict");
    }

    #[test]
    fn planner_uses_none_for_local_newer_before_two_way_ready() {
        let profile_id = test_profile_id("pre-two-way");
        clear_profile_rows(&profile_id);

        insert_sync_file_row(
            &profile_id,
            "local-newer-before-two-way.txt",
            true,
            true,
            100,
            200,
            10,
            10,
            Some("local-before-id"),
        );

        let counters = recompute_sync_file_actions(&profile_id, false).expect("recompute planner actions");
        assert_eq!(counters.need_upload_total, 0);

        let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
        let action: String = connection
            .query_row(
                "SELECT desired_action FROM sync_files WHERE profile_id = ?1 AND path = ?2",
                params![&profile_id, "local-newer-before-two-way.txt"],
                |row| row.get(0),
            )
            .expect("read desired action");
        assert_eq!(action, "none");
    }

    #[test]
    fn materialized_job_count_reports_active_download_and_upload_rows() {
        let profile_id = test_profile_id("materialized-counts");
        clear_profile_rows(&profile_id);
        let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
        let now = current_unix_seconds();

        connection
            .execute(
                "INSERT INTO sync_jobs (
                    profile_id, direction, item_id, path, remote_size, remote_modified_ts,
                    state, run_state, attempt_count, created_at, updated_at
                ) VALUES (?1, 'download', 'dl-id', 'dl.txt', 1, 1, 'queued', 'idle', 0, ?2, ?2)",
                params![&profile_id, now],
            )
            .expect("insert download job");
        connection
            .execute(
                "INSERT INTO sync_jobs (
                    profile_id, direction, item_id, path, remote_size, remote_modified_ts,
                    state, run_state, attempt_count, created_at, updated_at
                ) VALUES (?1, 'upload', 'up-id', 'up.txt', 1, 1, 'retry_wait', 'idle', 0, ?2, ?2)",
                params![&profile_id, now],
            )
            .expect("insert upload job");

        let (downloads, uploads) = read_materialized_job_counts(&profile_id).expect("read job counts");
        assert_eq!(downloads, 1);
        assert_eq!(uploads, 1);
    }

    #[test]
    fn planner_excludes_shared_references_from_actions() {
        let profile_id = test_profile_id("shared-ref");
        clear_profile_rows(&profile_id);

        insert_sync_file_row(
            &profile_id,
            "shared-item.txt",
            true,
            true,
            200,
            100,
            10,
            10,
            Some("shared-id"),
        );
        set_shared_reference(&profile_id, "shared-item.txt");

        let counters = recompute_sync_file_actions(&profile_id, true).expect("recompute planner actions");
        assert_eq!(counters.need_download_total, 0);
        assert_eq!(counters.need_upload_total, 0);
        assert_eq!(counters.shared_reference_total, 1);

        let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
        let action: String = connection
            .query_row(
                "SELECT desired_action FROM sync_files WHERE profile_id = ?1 AND path = ?2",
                params![&profile_id, "shared-item.txt"],
                |row| row.get(0),
            )
            .expect("read desired action");
        assert_eq!(action, "none");
    }

    #[test]
    fn planner_derives_delete_actions_from_previous_presence() {
        let profile_id = test_profile_id("delete-actions");
        clear_profile_rows(&profile_id);

        insert_sync_file_row(
            &profile_id,
            "local-deleted.txt",
            true,
            false,
            200,
            150,
            10,
            10,
            Some("local-deleted-id"),
        );
        insert_sync_file_row(
            &profile_id,
            "remote-deleted.txt",
            false,
            true,
            150,
            100,
            10,
            10,
            Some("remote-deleted-id"),
        );
        insert_sync_file_row(
            &profile_id,
            "remote-deleted-local-newer.txt",
            false,
            true,
            150,
            250,
            10,
            10,
            Some("remote-deleted-local-newer-id"),
        );

        let counters = recompute_sync_file_actions(&profile_id, true).expect("recompute planner actions");
        assert_eq!(counters.need_delete_remote_total, 1);
        assert_eq!(counters.need_delete_local_total, 1);
        assert_eq!(counters.need_upload_total, 1);

        let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
        let action_for_path = |path: &str| -> String {
            connection
                .query_row(
                    "SELECT desired_action FROM sync_files WHERE profile_id = ?1 AND path = ?2",
                    params![&profile_id, path],
                    |row| row.get::<_, String>(0),
                )
                .expect("read desired action")
        };

        assert_eq!(action_for_path("local-deleted.txt"), "delete_remote");
        assert_eq!(action_for_path("remote-deleted.txt"), "delete_local");
        assert_eq!(
            action_for_path("remote-deleted-local-newer.txt"),
            "upload"
        );
    }
}
