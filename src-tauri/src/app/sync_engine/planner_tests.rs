// Tests for the decision table (docs/sync-engine.md section 4) and for how
// delta pages and local scans are recorded.

#[cfg(test)]
mod planner_tests {
    use super::*;

    fn test_profile_id(label: &str) -> String {
        format!(
            "planner-test-{label}-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        )
    }

    /// One row's state for a planner test. `None` means absent on that side.
    #[derive(Default, Clone)]
    struct RowSpec {
        is_dir: bool,
        cloud: Option<(u64, &'static str)>,
        local: Option<(u64, i64)>,
        base: Option<(u64, i64, u64, &'static str)>,
    }

    fn insert_row(profile_id: &str, path: &str, spec: &RowSpec) {
        let connection = open_sync_jobs_connection(profile_id).expect("open db");
        let (cloud_present, cloud_size, cloud_hash) = match spec.cloud {
            Some((size, hash)) => (1, size as i64, Some(hash)),
            None => (0, 0, None),
        };
        let (local_present, local_size, local_mtime_ns) = match spec.local {
            Some((size, mtime)) => (1, size as i64, mtime),
            None => (0, 0, 0),
        };
        let (base_present, base_size, base_mtime, base_cloud_size, base_hash) = match spec.base {
            Some((size, mtime, cloud_size, hash)) => (1, size as i64, mtime, cloud_size as i64, Some(hash)),
            None => (0, 0, 0, 0, None),
        };
        connection
            .execute(
                "INSERT INTO sync_files (
                    profile_id, path, is_dir, remote_item_id, cloud_present, cloud_size, cloud_hash,
                    local_present, local_size, local_mtime_ns,
                    base_present, base_size, base_local_mtime_ns, base_cloud_size, base_hash, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, 0)",
                params![
                    profile_id,
                    path,
                    bool_to_sql(spec.is_dir),
                    (cloud_present == 1).then(|| format!("id-{path}")),
                    cloud_present,
                    cloud_size,
                    cloud_hash,
                    local_present,
                    local_size,
                    local_mtime_ns,
                    base_present,
                    base_size,
                    base_mtime,
                    base_cloud_size,
                    base_hash,
                ],
            )
            .expect("insert row");
    }

    fn action_for(profile_id: &str, path: &str) -> String {
        let connection = open_sync_jobs_connection(profile_id).expect("open db");
        connection
            .query_row(
                "SELECT desired_action FROM sync_files WHERE profile_id = ?1 AND path = ?2",
                params![profile_id, path],
                |row| row.get(0),
            )
            .expect("read action")
    }

    fn plan_single(spec: RowSpec) -> String {
        let profile_id = test_profile_id("row");
        insert_row(&profile_id, "file.txt", &spec);
        recompute_sync_file_actions(&profile_id).expect("plan");
        action_for(&profile_id, "file.txt")
    }

    const BASE: Option<(u64, i64, u64, &str)> = Some((10, 100, 10, "hash-a"));

    #[test]
    fn row_1_unchanged_on_both_sides_does_nothing() {
        let action = plan_single(RowSpec { cloud: Some((10, "hash-a")), local: Some((10, 100)), base: BASE, ..RowSpec::default() });
        assert_eq!(action, PLANNER_ACTION_NONE);
    }

    #[test]
    fn row_2_cloud_change_downloads() {
        let action = plan_single(RowSpec { cloud: Some((12, "hash-b")), local: Some((10, 100)), base: BASE, ..RowSpec::default() });
        assert_eq!(action, PLANNER_ACTION_DOWNLOAD);
    }

    #[test]
    fn row_3_local_change_uploads() {
        let action = plan_single(RowSpec { cloud: Some((10, "hash-a")), local: Some((10, 200)), base: BASE, ..RowSpec::default() });
        assert_eq!(action, PLANNER_ACTION_UPLOAD);
    }

    #[test]
    fn row_4_both_changed_is_a_conflict() {
        let action = plan_single(RowSpec { cloud: Some((12, "hash-b")), local: Some((11, 200)), base: BASE, ..RowSpec::default() });
        assert_eq!(action, PLANNER_ACTION_CONFLICT);
    }

    #[test]
    fn row_5_local_delete_deletes_in_cloud() {
        let action = plan_single(RowSpec { cloud: Some((10, "hash-a")), local: None, base: BASE, ..RowSpec::default() });
        assert_eq!(action, PLANNER_ACTION_DELETE_REMOTE);
    }

    #[test]
    fn row_6_cloud_delete_deletes_locally() {
        let action = plan_single(RowSpec { cloud: None, local: Some((10, 100)), base: BASE, ..RowSpec::default() });
        assert_eq!(action, PLANNER_ACTION_DELETE_LOCAL);
    }

    #[test]
    fn row_7_cloud_edit_beats_local_delete() {
        let action = plan_single(RowSpec { cloud: Some((12, "hash-b")), local: None, base: BASE, ..RowSpec::default() });
        assert_eq!(action, PLANNER_ACTION_DOWNLOAD);
    }

    #[test]
    fn row_8_local_edit_beats_cloud_delete() {
        let action = plan_single(RowSpec { cloud: None, local: Some((11, 200)), base: BASE, ..RowSpec::default() });
        assert_eq!(action, PLANNER_ACTION_UPLOAD);
    }

    #[test]
    fn row_9_gone_from_both_sides_is_forgotten() {
        let action = plan_single(RowSpec { cloud: None, local: None, base: BASE, ..RowSpec::default() });
        assert_eq!(action, PLANNER_ACTION_FORGET);
    }

    #[test]
    fn row_10_new_local_file_uploads() {
        let action = plan_single(RowSpec { local: Some((10, 100)), ..RowSpec::default() });
        assert_eq!(action, PLANNER_ACTION_UPLOAD);
    }

    #[test]
    fn row_11_new_cloud_file_downloads() {
        let action = plan_single(RowSpec { cloud: Some((10, "hash-a")), ..RowSpec::default() });
        assert_eq!(action, PLANNER_ACTION_DOWNLOAD);
    }

    #[test]
    fn row_12_new_on_both_sides_is_verified_never_overwritten() {
        let action = plan_single(RowSpec { cloud: Some((10, "hash-a")), local: Some((10, 100)), ..RowSpec::default() });
        assert_eq!(action, PLANNER_ACTION_VERIFY);
    }

    #[test]
    fn sharepoint_size_change_after_upload_is_not_a_cloud_change() {
        // Local 10 bytes, cloud rewrote it to 14 bytes with its own hash; the
        // baseline recorded the cloud version as uploaded.
        let action = plan_single(RowSpec {
            cloud: Some((14, "hash-sp")),
            local: Some((10, 100)),
            base: Some((10, 100, 14, "hash-sp")),
            ..RowSpec::default()
        });
        assert_eq!(action, PLANNER_ACTION_NONE);
    }

    #[test]
    fn baseline_without_a_cloud_version_fetches_the_cloud_copy() {
        // After a conflict copy the baseline holds the local file only.
        let profile_id = test_profile_id("conflict-preserved");
        insert_row(&profile_id, "file.txt", &RowSpec { cloud: Some((12, "hash-b")), local: Some((11, 200)), ..RowSpec::default() });
        mark_conflict_preserved(&profile_id, "file.txt", &LocalEntry { is_dir: false, size: 11, mtime_ns: 200 })
            .expect("mark conflict");
        recompute_sync_file_actions(&profile_id).expect("plan");
        assert_eq!(action_for(&profile_id, "file.txt"), PLANNER_ACTION_DOWNLOAD);
    }

    #[test]
    fn rows_with_issues_and_the_drive_root_are_never_acted_on() {
        let profile_id = test_profile_id("excluded");
        insert_row(&profile_id, "bad:name.txt", &RowSpec { local: Some((10, 100)), ..RowSpec::default() });
        insert_row(&profile_id, "", &RowSpec { is_dir: true, cloud: Some((0, "")), ..RowSpec::default() });
        record_sync_file_issue(&profile_id, "bad:name.txt", "name_invalid", "invalid").expect("issue");
        recompute_sync_file_actions(&profile_id).expect("plan");
        assert_eq!(action_for(&profile_id, "bad:name.txt"), PLANNER_ACTION_NONE);
        assert_eq!(action_for(&profile_id, ""), PLANNER_ACTION_NONE);
    }

    #[test]
    fn folder_is_deleted_in_cloud_only_after_its_contents() {
        let profile_id = test_profile_id("folder-delete");
        let folder = RowSpec { is_dir: true, cloud: Some((0, "")), base: Some((0, 1, 0, "")), ..RowSpec::default() };
        insert_row(&profile_id, "docs", &folder);
        insert_row(&profile_id, "docs/a.txt", &RowSpec { cloud: Some((10, "hash-a")), base: BASE, ..RowSpec::default() });
        recompute_sync_file_actions(&profile_id).expect("plan");
        assert_eq!(action_for(&profile_id, "docs"), PLANNER_ACTION_NONE);
        assert_eq!(action_for(&profile_id, "docs/a.txt"), PLANNER_ACTION_DELETE_REMOTE);

        record_cloud_absent(&profile_id, "docs/a.txt").expect("child deleted");
        forget_sync_file(&profile_id, "docs/a.txt").expect("forget child");
        recompute_sync_file_actions(&profile_id).expect("replan");
        assert_eq!(action_for(&profile_id, "docs"), PLANNER_ACTION_DELETE_REMOTE);
    }

    #[test]
    fn sibling_folder_with_a_shared_prefix_is_not_a_child() {
        let profile_id = test_profile_id("prefix");
        let folder = RowSpec { is_dir: true, cloud: Some((0, "")), base: Some((0, 1, 0, "")), ..RowSpec::default() };
        insert_row(&profile_id, "docs", &folder);
        insert_row(&profile_id, "docs-archive/a.txt", &RowSpec { cloud: Some((10, "hash-a")), local: Some((10, 100)), base: BASE, ..RowSpec::default() });
        recompute_sync_file_actions(&profile_id).expect("plan");
        assert_eq!(action_for(&profile_id, "docs"), PLANNER_ACTION_DELETE_REMOTE);
    }

    fn graph_item(id: &str, name: &str, parent_id: Option<&str>, is_dir: bool) -> GraphDriveItem {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "name": name,
            "size": if is_dir { 0 } else { 5 },
            "folder": if is_dir { Some(serde_json::json!({})) } else { None },
            "file": if is_dir { None } else { Some(serde_json::json!({ "hashes": { "quickXorHash": format!("hash-{id}") } })) },
            "parentReference": parent_id.map(|parent| serde_json::json!({ "id": parent })),
            "cTag": format!("ctag-{id}"),
            "eTag": format!("etag-{id}"),
        }))
        .expect("graph item")
    }

    fn cloud_path_of(profile_id: &str, item_id: &str) -> Option<String> {
        let connection = open_sync_jobs_connection(profile_id).expect("open db");
        connection
            .query_row(
                "SELECT path FROM sync_files WHERE profile_id = ?1 AND remote_item_id = ?2 AND cloud_present = 1",
                params![profile_id, item_id],
                |row| row.get(0),
            )
            .optional()
            .expect("query")
    }

    #[test]
    fn delta_paths_come_from_parent_ids_even_within_one_page() {
        let profile_id = test_profile_id("delta-paths");
        let mut root: GraphDriveItem = graph_item("root", "root", None, true);
        root.root = Some(serde_json::json!({}));
        let items = vec![
            root,
            graph_item("f1", "Docs", Some("root"), true),
            graph_item("f2", "2024", Some("f1"), true),
            graph_item("x1", "report.pdf", Some("f2"), false),
        ];
        let summary = apply_delta_page(&profile_id, "test", &items, None).expect("apply page");
        assert_eq!(summary.skipped_unresolved, 0);
        assert_eq!(cloud_path_of(&profile_id, "x1").as_deref(), Some("Docs/2024/report.pdf"));
    }

    #[test]
    fn item_with_an_unknown_parent_is_skipped_not_put_at_the_root() {
        let profile_id = test_profile_id("delta-orphan");
        let items = vec![graph_item("x1", "report.pdf", Some("missing-parent"), false)];
        let summary = apply_delta_page(&profile_id, "test", &items, None).expect("apply page");
        assert_eq!(summary.skipped_unresolved, 1);
        assert_eq!(cloud_path_of(&profile_id, "x1"), None);
    }

    #[test]
    fn deleting_a_cloud_folder_marks_everything_under_it_deleted() {
        let profile_id = test_profile_id("delta-folder-delete");
        let mut root = graph_item("root", "root", None, true);
        root.root = Some(serde_json::json!({}));
        apply_delta_page(
            &profile_id,
            "test",
            &[root, graph_item("f1", "Docs", Some("root"), true), graph_item("x1", "a.txt", Some("f1"), false)],
            None,
        )
        .expect("seed");
        let deleted: GraphDriveItem =
            serde_json::from_value(serde_json::json!({ "id": "f1", "deleted": {} })).expect("deleted");
        apply_delta_page(&profile_id, "test", &[deleted], None).expect("delete");
        assert_eq!(cloud_path_of(&profile_id, "f1"), None);
        assert_eq!(cloud_path_of(&profile_id, "x1"), None);
    }

    #[test]
    fn a_cloud_rename_moves_cloud_state_and_keeps_the_old_local_baseline() {
        let profile_id = test_profile_id("delta-rename");
        let mut root = graph_item("root", "root", None, true);
        root.root = Some(serde_json::json!({}));
        apply_delta_page(
            &profile_id,
            "test",
            &[root, graph_item("f1", "Docs", Some("root"), true), graph_item("x1", "a.txt", Some("f1"), false)],
            None,
        )
        .expect("seed");
        commit_baseline(
            &profile_id,
            "Docs/a.txt",
            &CloudVersion { etag: None, ctag: Some("ctag-x1".into()), quick_xor_hash: Some("hash-x1".into()), size: 5 },
            &LocalEntry { is_dir: false, size: 5, mtime_ns: 1 },
        )
        .expect("baseline");

        apply_delta_page(&profile_id, "test", &[graph_item("f1", "Papers", Some("root"), true)], None)
            .expect("rename");
        assert_eq!(cloud_path_of(&profile_id, "x1").as_deref(), Some("Papers/a.txt"));
        recompute_sync_file_actions(&profile_id).expect("plan");
        assert_eq!(action_for(&profile_id, "Docs/a.txt"), PLANNER_ACTION_DELETE_LOCAL);
        assert_eq!(action_for(&profile_id, "Papers/a.txt"), PLANNER_ACTION_DOWNLOAD);
    }

    #[test]
    fn onenote_notebooks_and_their_contents_are_excluded() {
        let profile_id = test_profile_id("onenote");
        let mut root = graph_item("root", "root", None, true);
        root.root = Some(serde_json::json!({}));
        let mut notebook = graph_item("nb", "Notes", Some("root"), true);
        notebook.package = Some(serde_json::json!({ "type": "oneNote" }));
        apply_delta_page(&profile_id, "test", &[root, notebook, graph_item("s1", "Section.one", Some("nb"), false)], None)
            .expect("apply");
        recompute_sync_file_actions(&profile_id).expect("plan");
        assert_eq!(action_for(&profile_id, "Notes"), PLANNER_ACTION_NONE);
        assert_eq!(action_for(&profile_id, "Notes/Section.one"), PLANNER_ACTION_NONE);
    }

    #[test]
    fn held_conflicts_take_a_choice_and_other_problems_do_not() {
        let profile_id = test_profile_id("problems");
        let both = RowSpec { cloud: Some((12, "hash-b")), local: Some((11, 200)), base: BASE, ..RowSpec::default() };
        insert_row(&profile_id, "report.docx", &both);
        insert_row(&profile_id, "bad:name.txt", &RowSpec { local: Some((5, 1)), ..RowSpec::default() });
        record_sync_file_issue(&profile_id, "report.docx", ISSUE_CONFLICT_BOTH_CHANGED, "both changed").expect("hold");
        record_sync_file_issue(&profile_id, "bad:name.txt", "name_invalid", "invalid").expect("issue");

        recompute_sync_file_actions(&profile_id).expect("plan");
        assert_eq!(action_for(&profile_id, "report.docx"), PLANNER_ACTION_NONE, "a held file is not synced");

        let recorded = set_conflict_resolution(
            &profile_id,
            &["report.docx".to_string(), "bad:name.txt".to_string()],
            RESOLUTION_KEEP_CLOUD,
        )
        .expect("resolve");
        assert_eq!(recorded, 1, "only the held conflict takes a choice");
        let choices = list_conflict_resolutions(&profile_id).expect("choices");
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0].resolution, RESOLUTION_KEEP_CLOUD);
        assert_eq!(choices[0].chosen_local_mtime_ns, Some(200));

        let problems = list_file_problems(&profile_id).expect("problems");
        assert_eq!(problems.len(), 2);
        let conflict = problems.iter().find(|problem| problem.path == "report.docx").expect("conflict");
        assert!(conflict.is_conflict);
        assert_eq!(conflict.local_size, Some(11));
        assert_eq!(conflict.cloud_size, Some(12));
        assert!(!problems.iter().find(|problem| problem.path == "bad:name.txt").expect("name").is_conflict);

        assert!(set_conflict_resolution(&profile_id, &["report.docx".to_string()], "delete_everything").is_err());

        clear_sync_file_issue(&profile_id, "report.docx").expect("clear");
        recompute_sync_file_actions(&profile_id).expect("replan");
        assert_eq!(action_for(&profile_id, "report.docx"), PLANNER_ACTION_CONFLICT, "unheld, the plan sees the conflict again");
    }

    #[test]
    fn failed_cycles_back_off_up_to_fifteen_minutes() {
        assert_eq!(sync_retry_delay(1), Duration::from_secs(30));
        assert_eq!(sync_retry_delay(2), Duration::from_secs(60));
        assert_eq!(sync_retry_delay(3), Duration::from_secs(120));
        assert_eq!(sync_retry_delay(20), Duration::from_secs(15 * 60));
    }

    #[test]
    fn a_delta_resync_lists_items_the_fresh_enumeration_did_not_return() {
        let profile_id = test_profile_id("resync");
        let mut root = graph_item("root", "root", None, true);
        root.root = Some(serde_json::json!({}));
        apply_delta_page(
            &profile_id,
            "test",
            &[root, graph_item("x1", "kept.txt", Some("root"), false), graph_item("x2", "gone.txt", Some("root"), false)],
            Some("https://example.test/next"),
        )
        .expect("seed");
        // Make the seeded rows look older than the resync.
        open_sync_jobs_connection(&profile_id)
            .expect("db")
            .execute("UPDATE sync_files SET cloud_seen_at = 1 WHERE profile_id = ?1", params![profile_id])
            .expect("age rows");

        let started = begin_delta_resync(&profile_id).expect("begin");
        assert_eq!(read_delta_resync_started_at(&profile_id).expect("marker"), Some(started));
        assert_eq!(read_sync_lifecycle_operational_state(&profile_id).expect("state").active_delta_next_link, None);

        let mut root = graph_item("root", "root", None, true);
        root.root = Some(serde_json::json!({}));
        apply_delta_page(&profile_id, "test", &[root, graph_item("x1", "kept.txt", Some("root"), false)], None)
            .expect("fresh enumeration");
        let unseen = list_cloud_items_unseen_since(&profile_id, started).expect("unseen");
        assert_eq!(unseen, vec![("gone.txt".to_string(), "x2".to_string())]);

        record_cloud_item_deleted(&profile_id, "x2").expect("confirm deleted");
        finish_delta_resync(&profile_id).expect("finish");
        assert_eq!(cloud_path_of(&profile_id, "x2"), None);
        assert_eq!(read_delta_resync_started_at(&profile_id).expect("marker"), None);
    }

    #[test]
    fn local_names_onedrive_rejects_become_issues() {
        assert!(invalid_onedrive_name_reason("report:final.docx").is_some());
        assert!(invalid_onedrive_name_reason("trailing.").is_some());
        assert!(invalid_onedrive_name_reason(" leading").is_some());
        assert!(invalid_onedrive_name_reason("CON.txt").is_some());
        assert!(invalid_onedrive_name_reason("com1").is_some());
        assert!(invalid_onedrive_name_reason("desktop.ini").is_some());
        assert!(invalid_onedrive_name_reason("Budget 2024.xlsx").is_none());
        assert!(invalid_onedrive_name_reason("console.log").is_none());
    }
}
