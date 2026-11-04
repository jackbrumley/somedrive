use super::*;

// Tests for how delta pages, renames, resyncs and exclusions are recorded.

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
    assert_eq!(
        cloud_path_of(&profile_id, "x1").as_deref(),
        Some("Docs/2024/report.pdf")
    );
}

#[test]
fn item_with_an_unknown_parent_is_skipped_not_put_at_the_root() {
    let profile_id = test_profile_id("delta-orphan");
    let items = vec![graph_item(
        "x1",
        "report.pdf",
        Some("missing-parent"),
        false,
    )];
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
        &[
            root,
            graph_item("f1", "Docs", Some("root"), true),
            graph_item("x1", "a.txt", Some("f1"), false),
        ],
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
fn a_cloud_rename_is_repeated_on_disk_without_downloading_again() {
    let profile_id = test_profile_id("rename-follow");
    seed_synced_folder(&profile_id);
    let sync_root = std::env::temp_dir().join(format!("somedrive-move-{}", profile_id));
    std::fs::create_dir_all(sync_root.join("Docs")).expect("folder");
    std::fs::write(sync_root.join("Docs/a.txt"), b"hello").expect("file");

    apply_delta_page(
        &profile_id,
        "test",
        &[graph_item("f1", "Papers", Some("root"), true)],
        None,
    )
    .expect("rename");
    assert_eq!(
        list_pending_local_moves(&profile_id).expect("moves"),
        vec![PendingLocalMove {
            old_path: "Docs".into(),
            new_path: "Papers".into()
        }]
    );
    assert_eq!(
        apply_pending_local_moves(&profile_id, "test", &sync_root).expect("apply"),
        1
    );
    assert!(sync_root.join("Papers/a.txt").is_file());
    assert!(!sync_root.join("Docs").exists());

    recompute_sync_file_actions(&profile_id).expect("plan");
    assert_eq!(
        action_for(&profile_id, "Papers/a.txt"),
        PLANNER_ACTION_NONE,
        "nothing to download again"
    );
    assert!(read_sync_file_row(&profile_id, "Docs/a.txt")
        .expect("row")
        .is_none());
    std::fs::remove_dir_all(&sync_root).expect("clean up");
}

#[test]
fn a_rename_onto_a_taken_path_falls_back_to_download_and_guarded_delete() {
    let profile_id = test_profile_id("rename-blocked");
    seed_synced_folder(&profile_id);
    let sync_root = std::env::temp_dir().join(format!("somedrive-move-{}", profile_id));
    std::fs::create_dir_all(sync_root.join("Docs")).expect("folder");
    std::fs::write(sync_root.join("Docs/a.txt"), b"hello").expect("file");
    std::fs::create_dir_all(sync_root.join("Papers")).expect("taken destination");

    apply_delta_page(
        &profile_id,
        "test",
        &[graph_item("f1", "Papers", Some("root"), true)],
        None,
    )
    .expect("rename");
    assert_eq!(
        apply_pending_local_moves(&profile_id, "test", &sync_root).expect("apply"),
        0
    );
    assert!(list_pending_local_moves(&profile_id)
        .expect("moves")
        .is_empty());
    assert!(
        sync_root.join("Docs/a.txt").is_file(),
        "the old copy is left alone"
    );

    recompute_sync_file_actions(&profile_id).expect("plan");
    assert_eq!(
        action_for(&profile_id, "Docs/a.txt"),
        PLANNER_ACTION_DELETE_LOCAL
    );
    assert_eq!(
        action_for(&profile_id, "Papers/a.txt"),
        PLANNER_ACTION_DOWNLOAD
    );
    std::fs::remove_dir_all(&sync_root).expect("clean up");
}

#[test]
fn onenote_notebooks_and_their_contents_are_excluded() {
    let profile_id = test_profile_id("onenote");
    let mut root = graph_item("root", "root", None, true);
    root.root = Some(serde_json::json!({}));
    let mut notebook = graph_item("nb", "Notes", Some("root"), true);
    notebook.package = Some(serde_json::json!({ "type": "oneNote" }));
    apply_delta_page(
        &profile_id,
        "test",
        &[
            root,
            notebook,
            graph_item("s1", "Section.one", Some("nb"), false),
        ],
        None,
    )
    .expect("apply");
    recompute_sync_file_actions(&profile_id).expect("plan");
    assert_eq!(action_for(&profile_id, "Notes"), PLANNER_ACTION_NONE);
    assert_eq!(
        action_for(&profile_id, "Notes/Section.one"),
        PLANNER_ACTION_NONE
    );
}

#[test]
fn a_delta_resync_lists_items_the_fresh_enumeration_did_not_return() {
    let profile_id = test_profile_id("resync");
    let mut root = graph_item("root", "root", None, true);
    root.root = Some(serde_json::json!({}));
    apply_delta_page(
        &profile_id,
        "test",
        &[
            root,
            graph_item("x1", "kept.txt", Some("root"), false),
            graph_item("x2", "gone.txt", Some("root"), false),
        ],
        Some("https://example.test/next"),
    )
    .expect("seed");
    // Make the seeded rows look older than the resync.
    open_sync_jobs_connection(&profile_id)
        .expect("db")
        .execute(
            "UPDATE sync_files SET cloud_seen_at = 1 WHERE profile_id = ?1",
            params![profile_id],
        )
        .expect("age rows");

    let started = begin_delta_resync(&profile_id).expect("begin");
    assert_eq!(
        read_delta_resync_started_at(&profile_id).expect("marker"),
        Some(started)
    );
    assert_eq!(
        read_sync_lifecycle_operational_state(&profile_id)
            .expect("state")
            .active_delta_next_link,
        None
    );

    let mut root = graph_item("root", "root", None, true);
    root.root = Some(serde_json::json!({}));
    apply_delta_page(
        &profile_id,
        "test",
        &[root, graph_item("x1", "kept.txt", Some("root"), false)],
        None,
    )
    .expect("fresh enumeration");
    let unseen = list_cloud_items_unseen_since(&profile_id, started).expect("unseen");
    assert_eq!(unseen, vec![("gone.txt".to_string(), "x2".to_string())]);

    record_cloud_item_deleted(&profile_id, "x2").expect("confirm deleted");
    finish_delta_resync(&profile_id).expect("finish");
    assert_eq!(cloud_path_of(&profile_id, "x2"), None);
    assert_eq!(
        read_delta_resync_started_at(&profile_id).expect("marker"),
        None
    );
}

#[test]
fn excluded_items_are_never_deleted_and_come_back_when_the_rule_goes() {
    let profile_id = test_profile_id("excluded-rule");
    // Synced before; now the scan skips it, so it looks deleted locally.
    insert_row(
        &profile_id,
        "Cache/blob.bin",
        &RowSpec {
            cloud: Some((10, "hash-a")),
            local: None,
            base: BASE,
            ..RowSpec::default()
        },
    );
    let rules = ExclusionRules::compile(&["Cache".to_string()]).expect("rules");
    assert_eq!(
        apply_exclusion_rules(&profile_id, &rules).expect("apply"),
        1
    );
    recompute_sync_file_actions(&profile_id).expect("plan");
    assert_eq!(
        action_for(&profile_id, "Cache/blob.bin"),
        PLANNER_ACTION_NONE,
        "excluded, so never deleted in OneDrive"
    );
    assert!(
        list_file_problems(&profile_id)
            .expect("problems")
            .is_empty(),
        "exclusions are not problems"
    );

    let no_rules = ExclusionRules::compile(&[]).expect("no rules");
    apply_exclusion_rules(&profile_id, &no_rules).expect("clear");
    recompute_sync_file_actions(&profile_id).expect("replan");
    assert_eq!(
        action_for(&profile_id, "Cache/blob.bin"),
        PLANNER_ACTION_DELETE_REMOTE
    );
}
