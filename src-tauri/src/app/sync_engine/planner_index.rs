fn read_sync_file_planner_counters(profile_id: &str) -> Result<SyncFilePlannerCounters, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let count_sync_files = |condition: &str, label: &str| -> Result<usize, String> {
        connection
            .query_row(
                &format!("SELECT COUNT(1) FROM sync_files WHERE profile_id = ?1 AND {condition}"),
                params![profile_id],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count.max(0) as usize)
            .map_err(|error| format!("Failed reading {label} counter: {error}"))
    };
    let mut counters = SyncFilePlannerCounters {
        cloud_discovered_total: count_sync_files("cloud_present = 1", "cloud discovered")?,
        local_discovered_total: count_sync_files("local_present = 1", "local discovered")?,
        shared_reference_total: count_sync_files("is_shared_reference = 1", "shared reference")?,
        ..SyncFilePlannerCounters::default()
    };

    let mut statement = connection
        .prepare(
            "SELECT desired_action, COUNT(1)
             FROM sync_files
             WHERE profile_id = ?1
             GROUP BY desired_action",
        )
        .map_err(|error| format!("Failed preparing sync file action counters query: {error}"))?;

    let rows = statement
        .query_map(params![profile_id], |row| {
            let action: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((action, count.max(0) as usize))
        })
        .map_err(|error| format!("Failed querying sync file action counters: {error}"))?;

    for row in rows {
        let (action, count) =
            row.map_err(|error| format!("Failed reading sync file action counters: {error}"))?;
        match action.as_str() {
            PLANNER_ACTION_NONE => counters.none_total = count,
            PLANNER_ACTION_DOWNLOAD => counters.need_download_total = count,
            PLANNER_ACTION_UPLOAD => counters.need_upload_total = count,
            PLANNER_ACTION_DELETE_REMOTE => counters.need_delete_remote_total = count,
            PLANNER_ACTION_DELETE_LOCAL => counters.need_delete_local_total = count,
            PLANNER_ACTION_CONFLICT => counters.conflict_total = count,
            _ => {}
        }
    }

    Ok(counters)
}

fn list_sync_file_download_candidates(profile_id: &str) -> Result<Vec<PlannerDownloadCandidate>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let mut statement = connection
        .prepare(
            "SELECT remote_item_id, path, cloud_size, cloud_mtime
             FROM sync_files
             WHERE profile_id = ?1
               AND desired_action = ?2
               AND is_dir = 0
               AND is_shared_reference = 0
               AND cloud_present = 1
             ORDER BY path ASC",
        )
        .map_err(|error| format!("Failed preparing download candidate query: {error}"))?;

    let rows = statement
        .query_map(params![profile_id, PLANNER_ACTION_DOWNLOAD], |row| {
            let remote_item_id: Option<String> = row.get(0)?;
            Ok((
                remote_item_id,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?.max(0) as u64,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|error| format!("Failed querying download candidates: {error}"))?;

    let mut candidates: Vec<PlannerDownloadCandidate> = Vec::new();
    for row in rows {
        let (remote_item_id, path, remote_size, remote_modified_ts) =
            row.map_err(|error| format!("Failed reading download candidate: {error}"))?;
        let item_id = remote_item_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                format!(
                    "Download planner candidate missing remote_item_id path={}",
                    path
                )
            })?;
        candidates.push(PlannerDownloadCandidate {
            item_id,
            path,
            remote_size,
            remote_modified_ts,
        });
    }

    Ok(candidates)
}

fn list_sync_file_upload_candidates(profile_id: &str) -> Result<Vec<PlannerUploadCandidate>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let mut statement = connection
        .prepare(
            "SELECT path, local_size, local_mtime_ns
             FROM sync_files
             WHERE profile_id = ?1
               AND desired_action = ?2
               AND is_dir = 0
               AND is_shared_reference = 0
               AND local_present = 1
             ORDER BY path ASC",
        )
        .map_err(|error| format!("Failed preparing upload candidate query: {error}"))?;

    let rows = statement
        .query_map(params![profile_id, PLANNER_ACTION_UPLOAD], |row| {
            Ok(PlannerUploadCandidate {
                path: row.get(0)?,
                local_size: row.get::<_, i64>(1)?.max(0) as u64,
                local_mtime_ns: row.get(2)?,
            })
        })
        .map_err(|error| format!("Failed querying upload candidates: {error}"))?;

    let mut candidates: Vec<PlannerUploadCandidate> = Vec::new();
    for row in rows {
        candidates.push(row.map_err(|error| format!("Failed reading upload candidate: {error}"))?);
    }

    Ok(candidates)
}
#[derive(Debug, Clone)]
struct PlannerDownloadCandidate {
    item_id: String,
    path: String,
    remote_size: u64,
    remote_modified_ts: i64,
}

#[derive(Debug, Clone)]
struct PlannerUploadCandidate {
    path: String,
    local_size: u64,
    local_mtime_ns: i64,
}
