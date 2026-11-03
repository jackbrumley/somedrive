fn upsert_sync_lifecycle_row(profile_id: &str, row: &SyncLifecycleStateRow) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let now = current_unix_seconds();
    connection
        .execute(
            "INSERT INTO sync_lifecycle_state (
                 profile_id,
                 two_way_ready,
                 bootstrap_scan_initialized,
                 bootstrap_full_scan_completed,
                 delta_link,
                 active_delta_next_link,
                 last_cycle_at,
                 phase,
                 phase_message,
                 remote_scan_complete,
                 activity_stage,
                 activity_progress_mode,
                 activity_current,
                 activity_total,
                 activity_unit,
                 activity_detail,
                 activity_cycle_id,
                 activity_updated_at,
                 large_delete_guard_approved,
                 large_delete_pending_paths_json,
                 agent_state,
                 last_sync_at,
                 updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)
             ON CONFLICT(profile_id)
             DO UPDATE SET
                 two_way_ready = excluded.two_way_ready,
                 bootstrap_scan_initialized = excluded.bootstrap_scan_initialized,
                 bootstrap_full_scan_completed = excluded.bootstrap_full_scan_completed,
                 delta_link = excluded.delta_link,
                 active_delta_next_link = excluded.active_delta_next_link,
                 last_cycle_at = excluded.last_cycle_at,
                 phase = excluded.phase,
                 phase_message = excluded.phase_message,
                 remote_scan_complete = excluded.remote_scan_complete,
                 activity_stage = excluded.activity_stage,
                 activity_progress_mode = excluded.activity_progress_mode,
                 activity_current = excluded.activity_current,
                 activity_total = excluded.activity_total,
                 activity_unit = excluded.activity_unit,
                 activity_detail = excluded.activity_detail,
                 activity_cycle_id = excluded.activity_cycle_id,
                 activity_updated_at = excluded.activity_updated_at,
                 large_delete_guard_approved = excluded.large_delete_guard_approved,
                 large_delete_pending_paths_json = excluded.large_delete_pending_paths_json,
                 agent_state = excluded.agent_state,
                 last_sync_at = excluded.last_sync_at,
                 updated_at = excluded.updated_at",
            params![
                profile_id,
                bool_to_sql(row.two_way_ready),
                bool_to_sql(row.bootstrap_scan_initialized),
                bool_to_sql(row.bootstrap_full_scan_completed),
                row.delta_link,
                row.active_delta_next_link,
                row.last_cycle_at,
                row.phase,
                row.phase_message,
                bool_to_sql(row.remote_scan_complete),
                row.activity_stage,
                row.activity_progress_mode,
                row.activity_current.map(|value| value as i64),
                row.activity_total.map(|value| value as i64),
                row.activity_unit,
                row.activity_detail,
                row.activity_cycle_id,
                row.activity_updated_at,
                bool_to_sql(row.large_delete_guard_approved),
                row.large_delete_pending_paths_json,
                row.agent_state,
                row.last_sync_at,
                now,
            ],
        )
        .map_err(|error| format!("Failed upserting sync lifecycle row: {error}"))?;
    Ok(())
}

fn read_sync_lifecycle_row(profile_id: &str) -> Result<Option<SyncLifecycleStateRow>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .query_row(
            "SELECT two_way_ready,
                    bootstrap_scan_initialized,
                    bootstrap_full_scan_completed,
                    delta_link,
                    active_delta_next_link,
                    last_cycle_at,
                    phase,
                    phase_message,
                    remote_scan_complete,
                    activity_stage,
                    activity_progress_mode,
                    activity_current,
                    activity_total,
                    activity_unit,
                    activity_detail,
                    activity_cycle_id,
                    activity_updated_at,
                    large_delete_guard_approved,
                    large_delete_pending_paths_json,
                    agent_state,
                    last_sync_at
             FROM sync_lifecycle_state
             WHERE profile_id = ?1",
            params![profile_id],
            |row| {
                Ok(SyncLifecycleStateRow {
                    two_way_ready: sql_to_bool(row.get::<_, i64>(0)?),
                    bootstrap_scan_initialized: sql_to_bool(row.get::<_, i64>(1)?),
                    bootstrap_full_scan_completed: sql_to_bool(row.get::<_, i64>(2)?),
                    delta_link: row.get(3)?,
                    active_delta_next_link: row.get(4)?,
                    last_cycle_at: row.get(5)?,
                    phase: row.get(6)?,
                    phase_message: row.get(7)?,
                    remote_scan_complete: sql_to_bool(row.get::<_, i64>(8)?),
                    activity_stage: row.get(9)?,
                    activity_progress_mode: row.get(10)?,
                    activity_current: row.get::<_, Option<i64>>(11)?.map(|value| value.max(0) as usize),
                    activity_total: row.get::<_, Option<i64>>(12)?.map(|value| value.max(0) as usize),
                    activity_unit: row.get(13)?,
                    activity_detail: row.get(14)?,
                    activity_cycle_id: row.get(15)?,
                    activity_updated_at: row.get(16)?,
                    large_delete_guard_approved: sql_to_bool(row.get::<_, i64>(17)?),
                    large_delete_pending_paths_json: row.get(18)?,
                    agent_state: row.get(19)?,
                    last_sync_at: row.get(20)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("Failed reading sync lifecycle row: {error}"))
}

fn read_sync_lifecycle_operational_state(
    profile_id: &str,
) -> Result<SyncLifecycleOperationalState, String> {
    let row = read_sync_lifecycle_row(profile_id)?.unwrap_or_default();
    Ok(SyncLifecycleOperationalState {
        two_way_ready: row.two_way_ready,
        bootstrap_scan_initialized: row.bootstrap_scan_initialized,
        bootstrap_full_scan_completed: row.bootstrap_full_scan_completed,
        delta_link: row.delta_link,
        active_delta_next_link: row.active_delta_next_link,
        last_cycle_at: row.last_cycle_at,
    })
}

fn persist_sync_lifecycle_operational_state(
    profile_id: &str,
    state: &SyncLifecycleOperationalState,
) -> Result<(), String> {
    let mut row = read_sync_lifecycle_row(profile_id)?.unwrap_or_default();
    row.two_way_ready = state.two_way_ready;
    row.bootstrap_scan_initialized = state.bootstrap_scan_initialized;
    row.bootstrap_full_scan_completed = state.bootstrap_full_scan_completed;
    row.delta_link = state.delta_link.clone();
    row.active_delta_next_link = state.active_delta_next_link.clone();
    row.last_cycle_at = state.last_cycle_at.clone();
    upsert_sync_lifecycle_row(profile_id, &row)
}

fn persist_sync_lifecycle_phase(profile_id: &str, phase: &str, phase_message: &str) -> Result<(), String> {
    let mut row = read_sync_lifecycle_row(profile_id)?.unwrap_or_default();
    let progress_mode = match phase {
        "paused" | "idle" | "error" => "hidden",
        _ => "indeterminate",
    };
    row.phase = phase.to_string();
    row.phase_message = phase_message.to_string();
    row.activity_stage = phase.to_string();
    row.activity_progress_mode = progress_mode.to_string();
    row.activity_current = if phase == "scanning_local" { Some(0) } else { None };
    row.activity_total = None;
    row.activity_unit = None;
    row.activity_detail = Some(phase_message.to_string());
    row.activity_cycle_id = None;
    row.activity_updated_at = current_unix_seconds();
    upsert_sync_lifecycle_row(profile_id, &row)
}

fn persist_sync_lifecycle_remote_scan_complete(profile_id: &str, complete: bool) -> Result<(), String> {
    let mut row = read_sync_lifecycle_row(profile_id)?.unwrap_or_default();
    row.remote_scan_complete = complete;
    upsert_sync_lifecycle_row(profile_id, &row)
}

// the structured activity contract (stage, progress_mode, current, total, unit, detail, cycle_id) is passed field by field; bundling it into one struct is tracked in docs/TODO.md.
#[allow(clippy::too_many_arguments)]
fn persist_sync_lifecycle_activity(
    profile_id: &str,
    stage: &str,
    progress_mode: &str,
    current: Option<usize>,
    total: Option<usize>,
    unit: Option<&str>,
    detail: Option<&str>,
    cycle_id: Option<&str>,
) -> Result<(), String> {
    let mut row = read_sync_lifecycle_row(profile_id)?.unwrap_or_default();
    validate_sync_lifecycle_activity_write(
        profile_id,
        &row.phase,
        stage,
        progress_mode,
        current,
        total,
        unit,
        cycle_id,
    )?;
    row.activity_stage = stage.to_string();
    row.activity_progress_mode = progress_mode.to_string();
    row.activity_current = current;
    row.activity_total = total;
    row.activity_unit = unit.map(ToString::to_string);
    row.activity_detail = detail.map(ToString::to_string);
    row.activity_cycle_id = cycle_id.map(ToString::to_string);
    row.activity_updated_at = current_unix_seconds();
    upsert_sync_lifecycle_row(profile_id, &row)
}

// the structured activity contract (stage, progress_mode, current, total, unit, detail, cycle_id) is passed field by field; bundling it into one struct is tracked in docs/TODO.md.
#[allow(clippy::too_many_arguments)]
fn validate_sync_lifecycle_activity_write(
    profile_id: &str,
    phase: &str,
    stage: &str,
    progress_mode: &str,
    current: Option<usize>,
    total: Option<usize>,
    unit: Option<&str>,
    cycle_id: Option<&str>,
) -> Result<(), String> {
    if stage.trim().is_empty() {
        return Err(format!(
            "Invalid lifecycle activity write for profile '{}': stage cannot be empty.",
            profile_id
        ));
    }
    if cycle_id.is_some_and(|value| value.trim().is_empty()) {
        return Err(format!(
            "Invalid lifecycle activity write for profile '{}': cycle_id cannot be blank when provided.",
            profile_id
        ));
    }

    match progress_mode {
        "hidden" => {
            if current.is_some() || total.is_some() || unit.is_some() {
                return Err(format!(
                    "Invalid lifecycle activity write for profile '{}': hidden progress cannot include current/total/unit.",
                    profile_id
                ));
            }
        }
        "indeterminate" => {}
        "determinate" => {
            let current_value = current.ok_or_else(|| {
                format!(
                    "Invalid lifecycle activity write for profile '{}': determinate progress requires current.",
                    profile_id
                )
            })?;
            let total_value = total.ok_or_else(|| {
                format!(
                    "Invalid lifecycle activity write for profile '{}': determinate progress requires total.",
                    profile_id
                )
            })?;
            if current_value > total_value {
                return Err(format!(
                    "Invalid lifecycle activity write for profile '{}': determinate progress current ({}) exceeds total ({}).",
                    profile_id,
                    current_value,
                    total_value
                ));
            }
            if unit.is_none() {
                return Err(format!(
                    "Invalid lifecycle activity write for profile '{}': determinate progress requires unit.",
                    profile_id
                ));
            }
        }
        _ => {
            return Err(format!(
                "Invalid lifecycle activity write for profile '{}': unsupported progress_mode '{}'.",
                profile_id, progress_mode
            ));
        }
    }

    if matches!(phase, "paused" | "idle" | "error") && progress_mode != "hidden" {
        return Err(format!(
            "Invalid lifecycle activity write for profile '{}': phase '{}' requires hidden progress mode, found '{}'.",
            profile_id, phase, progress_mode
        ));
    }

    Ok(())
}

pub(crate) fn persist_sync_lifecycle_agent_state(profile_id: &str, agent_state: &str) -> Result<(), String> {
    let mut row = read_sync_lifecycle_row(profile_id)?.unwrap_or_default();
    row.agent_state = agent_state.to_string();
    upsert_sync_lifecycle_row(profile_id, &row)
}

pub(crate) fn persist_sync_lifecycle_last_sync_at(
    profile_id: &str,
    last_sync_at: Option<&str>,
) -> Result<(), String> {
    let mut row = read_sync_lifecycle_row(profile_id)?.unwrap_or_default();
    row.last_sync_at = last_sync_at.map(ToString::to_string);
    upsert_sync_lifecycle_row(profile_id, &row)
}

pub(crate) fn read_sync_lifecycle_profile_metadata(
    profile_id: &str,
) -> Result<Option<(String, Option<String>)>, String> {
    Ok(read_sync_lifecycle_row(profile_id)?.map(|row| (row.agent_state, row.last_sync_at)))
}

fn read_large_delete_guard_state(profile_id: &str) -> Result<LargeDeleteGuardState, String> {
    let row = read_sync_lifecycle_row(profile_id)?.unwrap_or_default();
    let pending_paths = serde_json::from_str::<Vec<String>>(&row.large_delete_pending_paths_json)
        .map_err(|error| format!("Failed decoding large delete pending paths JSON: {error}"))?;
    Ok(LargeDeleteGuardState {
        approved: row.large_delete_guard_approved,
        pending_paths,
    })
}

fn persist_large_delete_guard_state(
    profile_id: &str,
    guard_state: &LargeDeleteGuardState,
) -> Result<(), String> {
    let mut row = read_sync_lifecycle_row(profile_id)?.unwrap_or_default();
    row.large_delete_guard_approved = guard_state.approved;
    row.large_delete_pending_paths_json = serde_json::to_string(&guard_state.pending_paths)
        .map_err(|error| format!("Failed encoding large delete pending paths JSON: {error}"))?;
    upsert_sync_lifecycle_row(profile_id, &row)
}

pub(crate) fn read_sync_authority_row_counts(
    profile_id: &str,
) -> Result<(usize, usize, usize), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let lifecycle_rows = connection
        .query_row(
            "SELECT COUNT(1) FROM sync_lifecycle_state WHERE profile_id = ?1",
            params![profile_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Failed reading lifecycle row count: {error}"))?
        .max(0) as usize;
    let planner_rows = connection
        .query_row(
            "SELECT COUNT(1) FROM sync_files WHERE profile_id = ?1",
            params![profile_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Failed reading planner row count: {error}"))?
        .max(0) as usize;
    let job_rows = connection
        .query_row(
            "SELECT COUNT(1) FROM sync_jobs WHERE profile_id = ?1",
            params![profile_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Failed reading job row count: {error}"))?
        .max(0) as usize;
    Ok((lifecycle_rows, planner_rows, job_rows))
}
