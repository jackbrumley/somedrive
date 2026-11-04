use super::*;

// Shared fixtures for the engine's tests: rows in a chosen state, fake Graph
// items, and lookups.

pub(crate) fn test_profile_id(label: &str) -> String {
    format!(
        "planner-test-{label}-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    )
}

/// One row's state for a planner test. `None` means absent on that side.
#[derive(Default, Clone)]
pub(crate) struct RowSpec {
    pub(crate) is_dir: bool,
    pub(crate) cloud: Option<(u64, &'static str)>,
    pub(crate) local: Option<(u64, i64)>,
    pub(crate) base: Option<(u64, i64, u64, &'static str)>,
}

pub(crate) fn insert_row(profile_id: &str, path: &str, spec: &RowSpec) {
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
        Some((size, mtime, cloud_size, hash)) => {
            (1, size as i64, mtime, cloud_size as i64, Some(hash))
        }
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

pub(crate) fn action_for(profile_id: &str, path: &str) -> String {
    let connection = open_sync_jobs_connection(profile_id).expect("open db");
    connection
        .query_row(
            "SELECT desired_action FROM sync_files WHERE profile_id = ?1 AND path = ?2",
            params![profile_id, path],
            |row| row.get(0),
        )
        .expect("read action")
}

pub(crate) fn plan_single(spec: RowSpec) -> String {
    let profile_id = test_profile_id("row");
    insert_row(&profile_id, "file.txt", &spec);
    recompute_sync_file_actions(&profile_id).expect("plan");
    action_for(&profile_id, "file.txt")
}

pub(crate) const BASE: Option<(u64, i64, u64, &str)> = Some((10, 100, 10, "hash-a"));

pub(crate) fn graph_item(
    id: &str,
    name: &str,
    parent_id: Option<&str>,
    is_dir: bool,
) -> GraphDriveItem {
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

pub(crate) fn cloud_path_of(profile_id: &str, item_id: &str) -> Option<String> {
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

pub(crate) fn seed_synced_folder(profile_id: &str) {
    let mut root = graph_item("root", "root", None, true);
    root.root = Some(serde_json::json!({}));
    apply_delta_page(
        profile_id,
        "test",
        &[
            root,
            graph_item("f1", "Docs", Some("root"), true),
            graph_item("x1", "a.txt", Some("f1"), false),
        ],
        None,
    )
    .expect("seed");
    let entry = LocalEntry {
        is_dir: false,
        size: 5,
        mtime_ns: 1,
    };
    apply_local_scan(
        profile_id,
        &HashMap::from([
            (
                "Docs".to_string(),
                LocalEntry {
                    is_dir: true,
                    size: 0,
                    mtime_ns: 1,
                },
            ),
            ("Docs/a.txt".to_string(), entry.clone()),
        ]),
    )
    .expect("scan");
    commit_baseline(
        profile_id,
        "Docs/a.txt",
        &CloudVersion {
            etag: None,
            ctag: Some("ctag-x1".into()),
            quick_xor_hash: Some("hash-x1".into()),
            size: 5,
        },
        &entry,
    )
    .expect("baseline");
}
