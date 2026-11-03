fn split_parent_and_name(relative_path: &str) -> Result<(&str, &str), String> {
    let trimmed = relative_path.trim_matches('/');
    if trimmed.is_empty() {
        return Err("Remote folder path is empty".to_string());
    }
    if let Some((parent, name)) = trimmed.rsplit_once('/') {
        return Ok((parent, name));
    }
    Ok(("", trimmed))
}

fn encode_graph_path(path: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for segment in path.split('/') {
        if segment.is_empty() {
            continue;
        }
        parts.push(percent_encode(segment));
    }
    parts.join("/")
}

fn percent_encode(input: &str) -> String {
    let mut output = String::new();
    for byte in input.bytes() {
        let unreserved = matches!(
            byte,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~'
        );
        if unreserved {
            output.push(byte as char);
        } else {
            output.push('%');
            output.push_str(&format!("{:02X}", byte));
        }
    }
    output
}

fn remote_known_item_from_drive_item(
    item: DriveItemResponse,
    fallback_path: &str,
) -> Result<RemoteKnownItem, String> {
    let path = if let (Some(name), Some(parent_ref)) =
        (item.name.clone(), item.parent_reference.clone())
    {
        let parent = parent_ref
            .path
            .as_deref()
            .map(extract_root_relative)
            .unwrap_or_default();
        normalize_relative_path(&if parent.is_empty() {
            name
        } else {
            format!("{}/{}", parent, name)
        })
    } else {
        fallback_path.to_string()
    };
    if path.trim().is_empty() {
        return Err("Remote item path cannot be empty".to_string());
    }

    Ok(RemoteKnownItem {
        id: item.id,
        path,
        is_dir: item.folder.is_some(),
        size: item.size.unwrap_or(0),
        modified_ts: parse_rfc3339_seconds(item.last_modified_date_time.as_deref()),
        is_shared_reference: false,
        shared_drive_id: None,
        shared_item_id: None,
        shared_kind: None,
    })
}

fn collect_local_snapshot(sync_root: &Path) -> Result<HashMap<String, LocalSnapshotEntry>, String> {
    collect_local_snapshot_with_progress(sync_root, |_| {})
}

fn collect_local_snapshot_with_progress<F>(
    sync_root: &Path,
    mut on_progress: F,
) -> Result<HashMap<String, LocalSnapshotEntry>, String>
where
    F: FnMut(&str),
{
    let mut snapshot = HashMap::new();
    if !sync_root.exists() {
        return Ok(snapshot);
    }

    let mut stack: Vec<PathBuf> = vec![sync_root.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current).map_err(|error| {
            format!(
                "Failed reading directory '{}': {}",
                current.display(),
                error
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let metadata = entry.metadata().map_err(|error| {
                format!("Failed reading metadata '{}': {}", path.display(), error)
            })?;

            if !metadata.is_file() && !metadata.is_dir() {
                continue;
            }

            let relative = path
                .strip_prefix(sync_root)
                .map_err(|error| {
                    format!(
                        "Failed computing relative path '{}': {}",
                        path.display(),
                        error
                    )
                })?
                .to_string_lossy()
                .replace('\\', "/");
            let normalized = normalize_relative_path(&relative);
            if normalized.is_empty() {
                continue;
            }

            let modified_ts = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_secs() as i64)
                .unwrap_or(0);

            on_progress(&normalized);

            snapshot.insert(
                normalized,
                LocalSnapshotEntry {
                    is_dir: metadata.is_dir(),
                    size: if metadata.is_file() {
                        metadata.len()
                    } else {
                        0
                    },
                    modified_ts,
                },
            );

            if metadata.is_dir() {
                stack.push(path);
            }
        }
    }

    Ok(snapshot)
}

fn read_local_entry(path: &Path) -> Result<Option<LocalSnapshotEntry>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    let modified_ts = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_secs() as i64)
        .unwrap_or(0);
    Ok(Some(LocalSnapshotEntry {
        is_dir: metadata.is_dir(),
        size: if metadata.is_file() {
            metadata.len()
        } else {
            0
        },
        modified_ts,
    }))
}

fn load_sync_state(profile_id: &str) -> Result<PersistedSyncState, String> {
    if let Some(state_json) = read_sync_state_store(profile_id)? {
        Ok(serde_json::from_str::<PersistedSyncState>(&state_json)
            .map_err(|error| format!("Failed decoding persisted sync state: {error}"))?)
    } else {
        Ok(PersistedSyncState::default())
    }
}

fn save_sync_state(profile_id: &str, state: &PersistedSyncState) -> Result<(), String> {
    let text = serde_json::to_string_pretty(state)
        .map_err(|error| format!("Failed encoding sync state JSON: {error}"))?;
    write_sync_state_store(profile_id, &text)?;
    Ok(())
}

fn rebuild_sync_state_caches_from_db(
    profile_id: &str,
    state: &mut PersistedSyncState,
) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let mut statement = connection
        .prepare(
            "SELECT
                path,
                remote_item_id,
                remote_present,
                local_present,
                is_dir,
                remote_size,
                local_size,
                remote_modified_ts,
                local_modified_ts,
                is_shared_reference,
                shared_drive_id,
                shared_item_id,
                shared_kind
             FROM sync_files
             WHERE profile_id = ?1",
        )
        .map_err(|error| format!("Failed preparing sync_files cache rebuild query: {error}"))?;
    let rows = statement
        .query_map(params![profile_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                sql_to_bool(row.get::<_, i64>(2)?),
                sql_to_bool(row.get::<_, i64>(3)?),
                sql_to_bool(row.get::<_, i64>(4)?),
                row.get::<_, i64>(5)?.max(0) as u64,
                row.get::<_, i64>(6)?.max(0) as u64,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
                sql_to_bool(row.get::<_, i64>(9)?),
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, Option<String>>(12)?,
            ))
        })
        .map_err(|error| format!("Failed executing sync_files cache rebuild query: {error}"))?;

    let mut remote_by_id: HashMap<String, RemoteKnownItem> = HashMap::new();
    let mut remote_path_to_id: HashMap<String, String> = HashMap::new();
    let mut local_snapshot: HashMap<String, LocalSnapshotEntry> = HashMap::new();

    for row in rows {
        let (
            path,
            remote_item_id,
            remote_present,
            local_present,
            is_dir,
            remote_size,
            local_size,
            remote_modified_ts,
            local_modified_ts,
            is_shared_reference,
            shared_drive_id,
            shared_item_id,
            shared_kind,
        ) = row.map_err(|error| format!("Failed reading sync_files row for cache rebuild: {error}"))?;

        if remote_present {
            if let Some(item_id) = remote_item_id.as_ref() {
                if !item_id.trim().is_empty() {
                    remote_path_to_id.insert(path.clone(), item_id.clone());
                    remote_by_id.insert(
                        item_id.clone(),
                        RemoteKnownItem {
                            id: item_id.clone(),
                            path: path.clone(),
                            is_dir,
                            size: remote_size,
                            modified_ts: remote_modified_ts,
                            is_shared_reference,
                            shared_drive_id,
                            shared_item_id,
                            shared_kind,
                        },
                    );
                }
            }
        }

        if local_present {
            local_snapshot.insert(
                path,
                LocalSnapshotEntry {
                    is_dir,
                    size: local_size,
                    modified_ts: local_modified_ts,
                },
            );
        }
    }

    state.remote_by_id = remote_by_id;
    state.remote_path_to_id = remote_path_to_id;
    state.local_snapshot = local_snapshot;
    Ok(())
}

fn rebuild_sync_state_from_db(profile_id: &str) -> Result<(), String> {
    let mut sync_state = load_sync_state(profile_id)?;
    rebuild_sync_state_caches_from_db(profile_id, &mut sync_state)?;
    save_sync_state(profile_id, &sync_state)
}

#[cfg(test)]
mod path_state_tests {
    use super::*;

    fn test_profile_id(label: &str) -> String {
        format!(
            "path-state-test-{label}-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        )
    }

    #[test]
    fn rebuild_sync_state_from_db_reconstructs_caches_from_sync_files() {
        let profile_id = test_profile_id("rebuild-caches");
        let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
        connection
            .execute("DELETE FROM sync_files WHERE profile_id = ?1", params![&profile_id])
            .expect("clear sync_files");

        let now = current_unix_seconds();
        connection
            .execute(
                "INSERT INTO sync_files (
                    profile_id, path, is_dir, is_shared_reference,
                    shared_drive_id, shared_item_id, shared_kind,
                    remote_item_id, remote_present, local_present,
                    remote_size, local_size, remote_modified_ts, local_modified_ts,
                    desired_action, conflict_state, updated_at
                ) VALUES (
                    ?1, 'docs/file.txt', 0, 0,
                    NULL, NULL, NULL,
                    'remote-1', 1, 1,
                    128, 64, 200, 180,
                    'none', NULL, ?2
                )",
                params![&profile_id, now],
            )
            .expect("insert sync_files row");

        let dirty_state = PersistedSyncState {
            remote_by_id: HashMap::from([(
                "stale-remote".to_string(),
                RemoteKnownItem {
                    id: "stale-remote".to_string(),
                    path: "stale/path.txt".to_string(),
                    is_dir: false,
                    size: 1,
                    modified_ts: 1,
                    is_shared_reference: false,
                    shared_drive_id: None,
                    shared_item_id: None,
                    shared_kind: None,
                },
            )]),
            remote_path_to_id: HashMap::from([("stale/path.txt".to_string(), "stale-remote".to_string())]),
            local_snapshot: HashMap::from([(
                "stale/path.txt".to_string(),
                LocalSnapshotEntry {
                    is_dir: false,
                    size: 1,
                    modified_ts: 1,
                },
            )]),
        };
        save_sync_state(&profile_id, &dirty_state).expect("seed stale state cache");

        rebuild_sync_state_from_db(&profile_id).expect("rebuild sync state from db");

        let rebuilt = load_sync_state(&profile_id).expect("load rebuilt sync state");
        assert_eq!(rebuilt.remote_by_id.len(), 1);
        assert!(rebuilt.remote_by_id.contains_key("remote-1"));
        assert_eq!(
            rebuilt.remote_path_to_id.get("docs/file.txt").cloned(),
            Some("remote-1".to_string())
        );
        assert_eq!(rebuilt.local_snapshot.len(), 1);
        assert_eq!(
            rebuilt.local_snapshot.get("docs/file.txt").map(|entry| entry.size),
            Some(64)
        );
    }
}
