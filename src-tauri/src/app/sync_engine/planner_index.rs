use super::*;

/// The planner counters the UI shows, from one index-only scan
/// (`idx_sync_files_plan`).
pub(crate) fn read_sync_file_planner_counters(
    profile_id: &str,
) -> Result<SyncFilePlannerCounters, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let mut statement = connection
        .prepare(
            "SELECT desired_action,
                    COUNT(1),
                    COALESCE(SUM(cloud_present = 1), 0),
                    COALESCE(SUM(local_present = 1), 0),
                    COALESCE(SUM(is_shared_reference = 1), 0)
             FROM sync_files
             WHERE profile_id = ?1
             GROUP BY desired_action",
        )
        .map_err(|error| format!("Failed preparing sync file counters query: {error}"))?;
    let rows = statement
        .query_map(params![profile_id], |row| {
            let as_count = |value: i64| value.max(0) as usize;
            Ok((
                row.get::<_, String>(0)?,
                as_count(row.get(1)?),
                as_count(row.get(2)?),
                as_count(row.get(3)?),
                as_count(row.get(4)?),
            ))
        })
        .map_err(|error| format!("Failed querying sync file counters: {error}"))?;

    let mut counters = SyncFilePlannerCounters::default();
    for row in rows {
        let (action, count, cloud, local, shared) =
            row.map_err(|error| format!("Failed reading sync file counters: {error}"))?;
        counters.cloud_discovered_total += cloud;
        counters.local_discovered_total += local;
        counters.shared_reference_total += shared;
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

pub(crate) fn list_sync_file_upload_candidates(
    profile_id: &str,
) -> Result<Vec<PlannerUploadCandidate>, String> {
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
pub(crate) struct PlannerUploadCandidate {
    pub(crate) path: String,
    pub(crate) local_size: u64,
    pub(crate) local_mtime_ns: i64,
}
