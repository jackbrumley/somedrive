use super::*;

// Planning whole-folder cloud deletes (remote_deletes.rs).

const SYNCED_DIR_DELETED_HERE: RowSpec = RowSpec {
    is_dir: true,
    cloud: Some((0, "")),
    local: None,
    base: Some((0, 0, 0, "")),
};

fn deleted_here_file() -> RowSpec {
    RowSpec {
        cloud: Some((10, "hash-a")),
        base: BASE,
        ..RowSpec::default()
    }
}

fn seed(profile_id: &str, extra_kept: bool) {
    insert_row(profile_id, "Old", &SYNCED_DIR_DELETED_HERE);
    insert_row(profile_id, "Old/a.txt", &deleted_here_file());
    insert_row(profile_id, "Old/Sub", &SYNCED_DIR_DELETED_HERE);
    insert_row(profile_id, "Old/Sub/b.txt", &deleted_here_file());
    insert_row(profile_id, "Older.txt", &deleted_here_file());
    if extra_kept {
        // Excluded or otherwise not planned: still in the cloud folder.
        insert_row(
            profile_id,
            "Old/Sub/kept.txt",
            &RowSpec {
                cloud: Some((10, "hash-k")),
                ..RowSpec::default()
            },
        );
    }
    let connection = open_sync_jobs_connection(profile_id).expect("open");
    connection
        .execute(
            "UPDATE sync_files SET cloud_etag = 'etag-' || path WHERE profile_id = ?1",
            params![profile_id],
        )
        .expect("etags");
    recompute_sync_file_actions(profile_id).expect("plan");
}

#[test]
fn a_folder_deleted_here_with_all_its_content_goes_as_one() {
    let profile_id = test_profile_id("folder-delete-whole");
    seed(&profile_id, false);
    let planned =
        list_sync_file_rows_by_action(&profile_id, PLANNER_ACTION_DELETE_REMOTE).expect("rows");
    let (folders, remaining) = plan_folder_deletes(&profile_id, &planned).expect("plan");

    assert_eq!(folders.len(), 1, "only the top-most folder");
    assert_eq!(folders[0].folder.path, "Old");
    assert_eq!(folders[0].file_count, 2);
    assert_eq!(folders[0].file_bytes, 20);
    let fallback: Vec<&str> = folders[0]
        .rows
        .iter()
        .map(|row| row.path.as_str())
        .collect();
    assert_eq!(fallback, vec!["Old/Sub/b.txt", "Old/a.txt"]);
    let rest: Vec<&str> = remaining.iter().map(|row| row.path.as_str()).collect();
    assert_eq!(rest, vec!["Older.txt"]);

    forget_folder_tree(&profile_id, "Old").expect("forget");
    let connection = open_sync_jobs_connection(&profile_id).expect("open");
    let left: i64 = connection
        .query_row(
            "SELECT COUNT(1) FROM sync_files WHERE profile_id = ?1",
            params![profile_id],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(left, 1, "only Older.txt stays tracked");
}

#[test]
fn a_folder_holding_anything_not_being_deleted_goes_item_by_item() {
    let profile_id = test_profile_id("folder-delete-partial");
    seed(&profile_id, true);
    let planned =
        list_sync_file_rows_by_action(&profile_id, PLANNER_ACTION_DELETE_REMOTE).expect("rows");
    let (folders, remaining) = plan_folder_deletes(&profile_id, &planned).expect("plan");
    assert!(folders.is_empty(), "Old/Sub still holds kept.txt");
    assert_eq!(remaining.len(), planned.len());
}

#[test]
fn inside_means_strictly_below_a_folder() {
    assert!(is_inside("Old/a.txt", "Old"));
    assert!(!is_inside("Old", "Old"));
    assert!(!is_inside("Older.txt", "Old"));
    assert!(!is_inside("Old/", "Old"));
}
