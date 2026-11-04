use super::*;

// Tests for the decision table (docs/sync-engine.md section 4) and for how
// delta pages and local scans are recorded.

#[test]
fn row_1_unchanged_on_both_sides_does_nothing() {
    let action = plan_single(RowSpec {
        cloud: Some((10, "hash-a")),
        local: Some((10, 100)),
        base: BASE,
        ..RowSpec::default()
    });
    assert_eq!(action, PLANNER_ACTION_NONE);
}

#[test]
fn row_2_cloud_change_downloads() {
    let action = plan_single(RowSpec {
        cloud: Some((12, "hash-b")),
        local: Some((10, 100)),
        base: BASE,
        ..RowSpec::default()
    });
    assert_eq!(action, PLANNER_ACTION_DOWNLOAD);
}

#[test]
fn row_3_local_change_uploads() {
    let action = plan_single(RowSpec {
        cloud: Some((10, "hash-a")),
        local: Some((10, 200)),
        base: BASE,
        ..RowSpec::default()
    });
    assert_eq!(action, PLANNER_ACTION_UPLOAD);
}

#[test]
fn row_4_both_changed_is_a_conflict() {
    let action = plan_single(RowSpec {
        cloud: Some((12, "hash-b")),
        local: Some((11, 200)),
        base: BASE,
        ..RowSpec::default()
    });
    assert_eq!(action, PLANNER_ACTION_CONFLICT);
}

#[test]
fn row_5_local_delete_deletes_in_cloud() {
    let action = plan_single(RowSpec {
        cloud: Some((10, "hash-a")),
        local: None,
        base: BASE,
        ..RowSpec::default()
    });
    assert_eq!(action, PLANNER_ACTION_DELETE_REMOTE);
}

#[test]
fn row_6_cloud_delete_deletes_locally() {
    let action = plan_single(RowSpec {
        cloud: None,
        local: Some((10, 100)),
        base: BASE,
        ..RowSpec::default()
    });
    assert_eq!(action, PLANNER_ACTION_DELETE_LOCAL);
}

#[test]
fn row_7_cloud_edit_beats_local_delete() {
    let action = plan_single(RowSpec {
        cloud: Some((12, "hash-b")),
        local: None,
        base: BASE,
        ..RowSpec::default()
    });
    assert_eq!(action, PLANNER_ACTION_DOWNLOAD);
}

#[test]
fn row_8_local_edit_beats_cloud_delete() {
    let action = plan_single(RowSpec {
        cloud: None,
        local: Some((11, 200)),
        base: BASE,
        ..RowSpec::default()
    });
    assert_eq!(action, PLANNER_ACTION_UPLOAD);
}

#[test]
fn row_9_gone_from_both_sides_is_forgotten() {
    let action = plan_single(RowSpec {
        cloud: None,
        local: None,
        base: BASE,
        ..RowSpec::default()
    });
    assert_eq!(action, PLANNER_ACTION_FORGET);
}

#[test]
fn row_10_new_local_file_uploads() {
    let action = plan_single(RowSpec {
        local: Some((10, 100)),
        ..RowSpec::default()
    });
    assert_eq!(action, PLANNER_ACTION_UPLOAD);
}

#[test]
fn row_11_new_cloud_file_downloads() {
    let action = plan_single(RowSpec {
        cloud: Some((10, "hash-a")),
        ..RowSpec::default()
    });
    assert_eq!(action, PLANNER_ACTION_DOWNLOAD);
}

#[test]
fn row_12_new_on_both_sides_is_verified_never_overwritten() {
    let action = plan_single(RowSpec {
        cloud: Some((10, "hash-a")),
        local: Some((10, 100)),
        ..RowSpec::default()
    });
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
    insert_row(
        &profile_id,
        "file.txt",
        &RowSpec {
            cloud: Some((12, "hash-b")),
            local: Some((11, 200)),
            ..RowSpec::default()
        },
    );
    mark_conflict_preserved(
        &profile_id,
        "file.txt",
        &LocalEntry {
            is_dir: false,
            size: 11,
            mtime_ns: 200,
        },
    )
    .expect("mark conflict");
    recompute_sync_file_actions(&profile_id).expect("plan");
    assert_eq!(action_for(&profile_id, "file.txt"), PLANNER_ACTION_DOWNLOAD);
}

#[test]
fn rows_with_issues_and_the_drive_root_are_never_acted_on() {
    let profile_id = test_profile_id("excluded");
    insert_row(
        &profile_id,
        "bad:name.txt",
        &RowSpec {
            local: Some((10, 100)),
            ..RowSpec::default()
        },
    );
    insert_row(
        &profile_id,
        "",
        &RowSpec {
            is_dir: true,
            cloud: Some((0, "")),
            ..RowSpec::default()
        },
    );
    record_sync_file_issue(&profile_id, "bad:name.txt", "name_invalid", "invalid").expect("issue");
    recompute_sync_file_actions(&profile_id).expect("plan");
    assert_eq!(action_for(&profile_id, "bad:name.txt"), PLANNER_ACTION_NONE);
    assert_eq!(action_for(&profile_id, ""), PLANNER_ACTION_NONE);
}

#[test]
fn folder_is_deleted_in_cloud_only_after_its_contents() {
    let profile_id = test_profile_id("folder-delete");
    let folder = RowSpec {
        is_dir: true,
        cloud: Some((0, "")),
        base: Some((0, 1, 0, "")),
        ..RowSpec::default()
    };
    insert_row(&profile_id, "docs", &folder);
    insert_row(
        &profile_id,
        "docs/a.txt",
        &RowSpec {
            cloud: Some((10, "hash-a")),
            base: BASE,
            ..RowSpec::default()
        },
    );
    recompute_sync_file_actions(&profile_id).expect("plan");
    assert_eq!(action_for(&profile_id, "docs"), PLANNER_ACTION_NONE);
    assert_eq!(
        action_for(&profile_id, "docs/a.txt"),
        PLANNER_ACTION_DELETE_REMOTE
    );

    record_cloud_absent(&profile_id, "docs/a.txt").expect("child deleted");
    forget_sync_file(&profile_id, "docs/a.txt").expect("forget child");
    recompute_sync_file_actions(&profile_id).expect("replan");
    assert_eq!(
        action_for(&profile_id, "docs"),
        PLANNER_ACTION_DELETE_REMOTE
    );
}

#[test]
fn sibling_folder_with_a_shared_prefix_is_not_a_child() {
    let profile_id = test_profile_id("prefix");
    let folder = RowSpec {
        is_dir: true,
        cloud: Some((0, "")),
        base: Some((0, 1, 0, "")),
        ..RowSpec::default()
    };
    insert_row(&profile_id, "docs", &folder);
    insert_row(
        &profile_id,
        "docs-archive/a.txt",
        &RowSpec {
            cloud: Some((10, "hash-a")),
            local: Some((10, 100)),
            base: BASE,
            ..RowSpec::default()
        },
    );
    recompute_sync_file_actions(&profile_id).expect("plan");
    assert_eq!(
        action_for(&profile_id, "docs"),
        PLANNER_ACTION_DELETE_REMOTE
    );
}

#[test]
fn held_conflicts_take_a_choice_and_other_problems_do_not() {
    let profile_id = test_profile_id("problems");
    let both = RowSpec {
        cloud: Some((12, "hash-b")),
        local: Some((11, 200)),
        base: BASE,
        ..RowSpec::default()
    };
    insert_row(&profile_id, "report.docx", &both);
    insert_row(
        &profile_id,
        "bad:name.txt",
        &RowSpec {
            local: Some((5, 1)),
            ..RowSpec::default()
        },
    );
    record_sync_file_issue(
        &profile_id,
        "report.docx",
        ISSUE_CONFLICT_BOTH_CHANGED,
        "both changed",
    )
    .expect("hold");
    record_sync_file_issue(&profile_id, "bad:name.txt", "name_invalid", "invalid").expect("issue");

    recompute_sync_file_actions(&profile_id).expect("plan");
    assert_eq!(
        action_for(&profile_id, "report.docx"),
        PLANNER_ACTION_NONE,
        "a held file is not synced"
    );

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
    let conflict = problems
        .iter()
        .find(|problem| problem.path == "report.docx")
        .expect("conflict");
    assert!(conflict.is_conflict);
    assert_eq!(conflict.local_size, Some(11));
    assert_eq!(conflict.cloud_size, Some(12));
    assert!(
        !problems
            .iter()
            .find(|problem| problem.path == "bad:name.txt")
            .expect("name")
            .is_conflict
    );

    assert!(set_conflict_resolution(
        &profile_id,
        &["report.docx".to_string()],
        "delete_everything"
    )
    .is_err());

    clear_sync_file_issue(&profile_id, "report.docx").expect("clear");
    recompute_sync_file_actions(&profile_id).expect("replan");
    assert_eq!(
        action_for(&profile_id, "report.docx"),
        PLANNER_ACTION_CONFLICT,
        "unheld, the plan sees the conflict again"
    );
}

#[test]
fn failed_cycles_back_off_up_to_fifteen_minutes() {
    assert_eq!(sync_retry_delay(1), Duration::from_secs(30));
    assert_eq!(sync_retry_delay(2), Duration::from_secs(60));
    assert_eq!(sync_retry_delay(3), Duration::from_secs(120));
    assert_eq!(sync_retry_delay(20), Duration::from_secs(15 * 60));
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
