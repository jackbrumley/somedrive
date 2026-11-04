fn persist_sync_issue(
    profile_id: &str,
    issue_code: &str,
    issue_message: &str,
    issue_actions: &[&str],
    issue_path: Option<&str>,
    issue_secondary_path: Option<&str>,
) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
    let issue_actions_json = serde_json::to_string(issue_actions)
        .map_err(|error| format!("Failed serializing issue actions: {error}"))?;
    connection
        .execute(
            "INSERT INTO sync_issue_state (
                profile_id,
                issue_code,
                issue_message,
                issue_actions_json,
                issue_path,
                issue_secondary_path,
                updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(profile_id)
             DO UPDATE SET
                issue_code = excluded.issue_code,
                issue_message = excluded.issue_message,
                issue_actions_json = excluded.issue_actions_json,
                issue_path = excluded.issue_path,
                issue_secondary_path = excluded.issue_secondary_path,
                updated_at = excluded.updated_at",
            params![
                profile_id,
                issue_code,
                issue_message,
                issue_actions_json,
                issue_path,
                issue_secondary_path,
                now,
            ],
        )
        .map_err(|error| format!("Failed persisting sync issue: {error}"))?;
    Ok(())
}

fn clear_persisted_sync_issue(profile_id: &str) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "DELETE FROM sync_issue_state WHERE profile_id = ?1",
            params![profile_id],
        )
        .map_err(|error| format!("Failed clearing sync issue state: {error}"))?;
    Ok(())
}

fn read_persisted_sync_issue(profile_id: &str) -> Result<Option<PersistedSyncIssue>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let row = connection
        .query_row(
            "SELECT issue_code, issue_message, issue_actions_json, issue_path, issue_secondary_path
             FROM sync_issue_state
             WHERE profile_id = ?1",
            params![profile_id],
            |row| {
                let issue_actions_json: String = row.get(2)?;
                let issue_actions = serde_json::from_str::<Vec<String>>(&issue_actions_json)
                    .unwrap_or_default();
                Ok(PersistedSyncIssue {
                    issue_code: row.get(0)?,
                    issue_message: row.get(1)?,
                    issue_actions,
                    issue_path: row.get(3)?,
                    issue_secondary_path: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("Failed reading sync issue state: {error}"))?;
    Ok(row)
}

fn record_throttle_event(profile_id: &str, direction: &str) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
    let trim_before = now.saturating_sub(3600);
    connection
        .execute(
            "INSERT INTO sync_throttle_events (profile_id, direction, occurred_at) VALUES (?1, ?2, ?3)",
            params![profile_id, direction, now],
        )
        .map_err(|error| format!("Failed recording throttle event: {error}"))?;
    connection
        .execute(
            "DELETE FROM sync_throttle_events WHERE profile_id = ?1 AND occurred_at < ?2",
            params![profile_id, trim_before],
        )
        .map_err(|error| format!("Failed trimming throttle events: {error}"))?;
    Ok(())
}

fn read_throttle_counters(profile_id: &str) -> Result<ThrottleCounters, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
    let last_minute = now.saturating_sub(60);
    connection
        .query_row(
            "SELECT
                SUM(CASE WHEN direction = ?2 THEN 1 ELSE 0 END),
                SUM(CASE WHEN direction = ?2 AND occurred_at >= ?4 THEN 1 ELSE 0 END),
                SUM(CASE WHEN direction = ?3 THEN 1 ELSE 0 END),
                SUM(CASE WHEN direction = ?3 AND occurred_at >= ?4 THEN 1 ELSE 0 END)
             FROM sync_throttle_events
             WHERE profile_id = ?1",
            params![
                profile_id,
                DOWNLOAD_JOB_DIRECTION,
                UPLOAD_JOB_DIRECTION,
                last_minute,
            ],
            |row| {
                Ok(ThrottleCounters {
                    download_total: row.get::<_, Option<i64>>(0)?.unwrap_or(0).max(0) as usize,
                    download_last_minute: row.get::<_, Option<i64>>(1)?.unwrap_or(0).max(0)
                        as usize,
                    upload_total: row.get::<_, Option<i64>>(2)?.unwrap_or(0).max(0) as usize,
                    upload_last_minute: row.get::<_, Option<i64>>(3)?.unwrap_or(0).max(0) as usize,
                })
            },
        )
        .map_err(|error| format!("Failed reading throttle counters: {error}"))
}

/// Events of `direction` recorded since `since_unix_seconds` (the delete
/// guard counts deletes over a rolling window with this).
fn count_events_since(profile_id: &str, direction: &str, since_unix_seconds: i64) -> Result<usize, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .query_row(
            "SELECT COUNT(1) FROM sync_throttle_events
             WHERE profile_id = ?1 AND direction = ?2 AND occurred_at >= ?3",
            params![profile_id, direction, since_unix_seconds],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count.max(0) as usize)
        .map_err(|error| format!("Failed counting {direction} events: {error}"))
}
