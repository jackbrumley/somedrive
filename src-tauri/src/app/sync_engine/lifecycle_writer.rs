use super::*;

pub(crate) fn runtime_set_phase(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    phase: &str,
    phase_message: &str,
) {
    let mut previous_phase: Option<String> = None;
    let mut previous_message: Option<String> = None;
    if let Ok(mut runtime_map) = runtime.lock() {
        if let Some(existing) = runtime_map.get(profile_id) {
            previous_phase = Some(existing.phase.clone());
            previous_message = Some(existing.phase_message.clone());
        }
        sync_runtime::set_phase(&mut runtime_map, profile_id, phase, phase_message);
    }
    let phase_changed = previous_phase.as_deref() != Some(phase)
        || previous_message.as_deref() != Some(phase_message);
    if phase_changed {
        log::info!(
            "{} PHASE_TRANSITION from_phase={} from_message={} to_phase={} to_message={}",
            log_context::account_prefix(profile_id),
            previous_phase.as_deref().unwrap_or("(none)"),
            previous_message.as_deref().unwrap_or("(none)"),
            phase,
            phase_message
        );
    }
    if let Err(error) = persist_sync_lifecycle_phase(profile_id, phase, phase_message) {
        log::warn!(
            "{} SYNC_LIFECYCLE_PHASE_PERSIST_FAILED phase={} error={}",
            log_context::account_prefix(profile_id),
            phase,
            error
        );
    }
}

pub fn runtime_set_profile_auth_ready(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    auth_ready: bool,
) {
    if let Ok(mut runtime_map) = runtime.lock() {
        sync_runtime::set_auth_ready(&mut runtime_map, profile_id, auth_ready);
    }
}

pub fn runtime_set_engine_state(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    engine_state: &str,
) {
    if let Ok(mut runtime_map) = runtime.lock() {
        sync_runtime::set_engine_state(&mut runtime_map, profile_id, engine_state);
    }
}

pub(crate) fn runtime_reset_transfer_activity(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
) {
    if let Ok(mut runtime_map) = runtime.lock() {
        sync_runtime::reset_transfer_activity(&mut runtime_map, profile_id);
    }
}

pub(crate) fn runtime_set_local_scan_progress(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    scanned_count: usize,
    estimated_total: Option<usize>,
    current_path: Option<&str>,
    cycle_id: Option<&str>,
) {
    if let Ok(mut runtime_map) = runtime.lock() {
        sync_runtime::set_local_scan_progress(
            &mut runtime_map,
            profile_id,
            scanned_count,
            estimated_total,
            current_path,
            cycle_id,
        );
    }
    if let Err(error) = persist_sync_lifecycle_activity(
        profile_id,
        "scanning_local",
        if estimated_total.is_some() {
            "determinate"
        } else {
            "indeterminate"
        },
        Some(scanned_count),
        estimated_total,
        Some("files"),
        current_path,
        cycle_id,
    ) {
        log::warn!(
            "{} SYNC_LIFECYCLE_ACTIVITY_PERSIST_FAILED stage=scanning_local error={}",
            log_context::account_prefix(profile_id),
            error
        );
    }
}

/// Records whether the account's folder is watched for changes, and
/// sends the status when it changed.
pub(crate) fn runtime_set_local_watch(profile_id: &str, watching: bool) {
    if let Err(error) = persist_local_watch_state(profile_id, watching) {
        log::warn!(
            "{} LOCAL_WATCH_STATE_PERSIST_FAILED error={}",
            log_context::account_prefix(profile_id),
            error
        );
    }
    sync_runtime::request_status_event(profile_id, sync_runtime::StatusEventEmitMode::Immediate);
}

// the structured activity contract (stage, progress_mode, current, total, unit, detail, cycle_id) is passed field by field; bundling it into one struct is tracked in docs/TODO.md.
#[allow(clippy::too_many_arguments)]
pub(crate) fn runtime_set_current_activity(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    stage: &str,
    progress_mode: &str,
    current: Option<usize>,
    total: Option<usize>,
    unit: Option<&str>,
    detail: Option<&str>,
    cycle_id: Option<&str>,
) {
    if let Ok(mut runtime_map) = runtime.lock() {
        sync_runtime::set_current_activity(
            &mut runtime_map,
            profile_id,
            stage,
            progress_mode,
            current,
            total,
            unit,
            detail,
            cycle_id,
        );
    }
    if let Err(error) = persist_sync_lifecycle_activity(
        profile_id,
        stage,
        progress_mode,
        current,
        total,
        unit,
        detail,
        cycle_id,
    ) {
        log::warn!(
            "{} SYNC_LIFECYCLE_ACTIVITY_PERSIST_FAILED stage={} error={}",
            log_context::account_prefix(profile_id),
            stage,
            error
        );
    }
}

pub(crate) fn runtime_clear_issue(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
) {
    if let Ok(mut runtime_map) = runtime.lock() {
        sync_runtime::clear_issue(&mut runtime_map, profile_id);
    }
    if let Err(error) = clear_persisted_sync_issue(profile_id) {
        log::warn!(
            "{} SYNC_ISSUE_CLEAR_PERSIST_FAILED error={}",
            log_context::account_prefix(profile_id),
            error
        );
    }
}

pub(crate) fn runtime_set_issue(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    issue_code: &str,
    issue_message: &str,
    issue_actions: &[&str],
    issue_path: Option<&str>,
    issue_secondary_path: Option<&str>,
) {
    if let Ok(mut runtime_map) = runtime.lock() {
        sync_runtime::set_issue(
            &mut runtime_map,
            profile_id,
            issue_code,
            issue_message,
            issue_actions,
            issue_path,
            issue_secondary_path,
        );
    }
    if let Err(error) = persist_sync_issue(
        profile_id,
        issue_code,
        issue_message,
        issue_actions,
        issue_path,
        issue_secondary_path,
    ) {
        log::warn!(
            "{} SYNC_ISSUE_PERSIST_FAILED code={} error={}",
            log_context::account_prefix(profile_id),
            issue_code,
            error
        );
    }
}

pub(crate) fn runtime_set_remote_scan_complete(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    complete: bool,
) {
    if let Ok(mut runtime_map) = runtime.lock() {
        sync_runtime::set_remote_scan_complete(&mut runtime_map, profile_id, complete);
    }
    if let Err(error) = persist_sync_lifecycle_remote_scan_complete(profile_id, complete) {
        log::warn!(
            "{} SYNC_LIFECYCLE_SCAN_COMPLETE_PERSIST_FAILED complete={} error={}",
            log_context::account_prefix(profile_id),
            complete,
            error
        );
    }
}

pub(crate) fn persist_lifecycle_operational_state(
    profile_id: &str,
    state: &SyncLifecycleOperationalState,
) -> Result<(), String> {
    persist_sync_lifecycle_operational_state(profile_id, state)
}

pub(crate) fn runtime_record_remote_download_completed(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    item_id: &str,
) {
    if let Ok(mut runtime_map) = runtime.lock() {
        sync_runtime::record_remote_download_completed(&mut runtime_map, profile_id, item_id);
    }
}

pub(crate) fn runtime_record_remote_download_failed(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    item_id: &str,
) {
    if let Ok(mut runtime_map) = runtime.lock() {
        sync_runtime::record_remote_download_failed(&mut runtime_map, profile_id, item_id);
    }
}

pub(crate) fn runtime_set_remote_download_counters(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    planned_total: usize,
    completed_total: usize,
    failed_total: usize,
    in_flight: usize,
    retry_waiting: usize,
) {
    if let Ok(mut runtime_map) = runtime.lock() {
        sync_runtime::set_remote_download_counters(
            &mut runtime_map,
            profile_id,
            planned_total,
            completed_total,
            failed_total,
            in_flight,
            retry_waiting,
        );
    }
}

pub(crate) fn runtime_set_upload_counters(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    planned_total: usize,
    completed_total: usize,
    failed_total: usize,
    in_flight: usize,
) {
    if let Ok(mut runtime_map) = runtime.lock() {
        sync_runtime::set_upload_counters(
            &mut runtime_map,
            profile_id,
            planned_total,
            completed_total,
            failed_total,
            in_flight,
        );
    }
}

pub(crate) fn runtime_set_upload_planned_total(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    planned_total: usize,
) {
    if let Ok(mut runtime_map) = runtime.lock() {
        sync_runtime::set_upload_planned_total(&mut runtime_map, profile_id, planned_total);
    }
}

pub(crate) fn runtime_profile_auth_ready(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
) -> bool {
    runtime
        .lock()
        .ok()
        .and_then(|runtime_map| runtime_map.get(profile_id).map(|status| status.auth_ready))
        .unwrap_or(false)
}

#[cfg(test)]
mod lifecycle_writer_tests {
    use super::*;
    use crate::app::sync_runtime::SyncRuntimeAccountStatus;

    fn test_profile_id(label: &str) -> String {
        format!(
            "lifecycle-writer-test-{label}-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        )
    }

    #[test]
    fn hydrate_runtime_rejects_non_hidden_progress_for_paused_phase() {
        let profile_id = test_profile_id("lifecycle-invariant");
        persist_sync_lifecycle_phase(&profile_id, "paused", "Synchronization paused")
            .expect("persist paused phase");
        let mut lifecycle_row = read_sync_lifecycle_row(&profile_id)
            .expect("read lifecycle row")
            .expect("lifecycle row exists");
        lifecycle_row.activity_stage = "paused".to_string();
        lifecycle_row.activity_progress_mode = "determinate".to_string();
        lifecycle_row.activity_current = Some(1);
        lifecycle_row.activity_total = Some(2);
        lifecycle_row.activity_unit = Some("items".to_string());
        lifecycle_row.activity_detail = Some("invalid paused progress".to_string());
        lifecycle_row.activity_cycle_id = Some("legacy-cycle".to_string());
        upsert_sync_lifecycle_row(&profile_id, &lifecycle_row)
            .expect("persist invalid lifecycle activity");

        let mut runtime_map = SyncRuntimeMap::new();
        sync_runtime::set_phase(
            &mut runtime_map,
            &profile_id,
            "paused",
            "Synchronization paused",
        );
        let mut status = runtime_map
            .remove(&profile_id)
            .expect("runtime account status exists");

        let error = hydrate_runtime_status_from_db(&mut status)
            .expect_err("paused phase with determinate progress must fail");
        assert!(error.contains("requires hidden progress mode"));
    }

    #[test]
    fn persist_activity_rejects_non_hidden_progress_for_paused_phase() {
        let profile_id = test_profile_id("activity-write-paused");
        persist_sync_lifecycle_phase(&profile_id, "paused", "Synchronization paused")
            .expect("persist paused phase");

        let error = persist_sync_lifecycle_activity(
            &profile_id,
            "paused",
            "determinate",
            Some(1),
            Some(2),
            Some("items"),
            Some("invalid paused write"),
            Some("cycle-1"),
        )
        .expect_err("paused phase must reject non-hidden progress writes");

        assert!(error.contains("requires hidden progress mode"));
    }

    #[test]
    fn persist_activity_rejects_invalid_determinate_progress_fields() {
        let profile_id = test_profile_id("activity-write-determinate");
        persist_sync_lifecycle_phase(
            &profile_id,
            "planning_actions",
            "Planning synchronization actions",
        )
        .expect("persist planning phase");

        let error = persist_sync_lifecycle_activity(
            &profile_id,
            "planning_actions",
            "determinate",
            Some(3),
            None,
            Some("items"),
            Some("invalid determinate write"),
            Some("cycle-2"),
        )
        .expect_err("determinate progress without total must fail");

        assert!(error.contains("determinate progress requires total"));
    }

    #[test]
    fn canonical_builder_matches_hydrated_status_for_same_profile() {
        let profile_id = test_profile_id("canonical-builder-equivalence");
        persist_sync_lifecycle_phase(&profile_id, "syncing", "Initial sync in progress")
            .expect("persist syncing phase");
        persist_sync_lifecycle_activity(
            &profile_id,
            "scanning_remote",
            "indeterminate",
            None,
            None,
            Some("items"),
            Some("Fetching remote files"),
            Some("cycle-equivalence"),
        )
        .expect("persist remote scan activity");

        let mut hydrated = SyncRuntimeAccountStatus::canonical_seed(&profile_id, true);
        hydrate_runtime_status_from_db(&mut hydrated).expect("hydrate runtime status");
        sync_runtime::recompute_authority_fields(&mut hydrated);

        let built = build_authoritative_runtime_status(&profile_id, true)
            .expect("build authoritative runtime status");

        assert_eq!(hydrated.profile_id, built.profile_id);
        assert_eq!(hydrated.phase, built.phase);
        assert_eq!(hydrated.phase_message, built.phase_message);
        assert_eq!(
            hydrated.current_activity.stage,
            built.current_activity.stage
        );
        assert_eq!(
            hydrated.current_activity.progress_mode,
            built.current_activity.progress_mode
        );
        assert_eq!(hydrated.remote_scan_complete, built.remote_scan_complete);
        assert_eq!(
            hydrated.remote_download_planned_total,
            built.remote_download_planned_total
        );
        assert_eq!(hydrated.upload_planned_total, built.upload_planned_total);
    }
}
