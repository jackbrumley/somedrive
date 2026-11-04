use super::*;

// Fields derived from an account's status (severity, can-sync, consistency
// checks) so every status the UI receives follows the same rules.

pub fn recompute_authority_fields(status: &mut SyncRuntimeAccountStatus) {
    ensure_auth_required_issue_contract(status);
    status.issue_severity = issue_severity_from_code(status.issue_code.as_deref()).to_string();
    status.can_sync =
        status.auth_ready && status.issue_severity != "blocking" && status.phase != "error";
    let violations = collect_consistency_violations(status);
    status.consistency = SyncRuntimeConsistency {
        ok: violations.is_empty(),
        violations,
    };
}

pub(crate) fn collect_consistency_violations(status: &SyncRuntimeAccountStatus) -> Vec<String> {
    let mut violations = Vec::new();
    let expected_download_settled = status
        .remote_download_completed_total
        .saturating_add(status.remote_download_failed_total)
        .saturating_add(status.remote_download_in_flight)
        .saturating_add(status.remote_download_retry_waiting);
    if expected_download_settled > status.remote_download_planned_total {
        violations.push("download_lane_overcommitted".to_string());
    }

    let expected_upload_settled = status
        .upload_completed_total
        .saturating_add(status.upload_failed_total)
        .saturating_add(status.upload_in_flight)
        .saturating_add(status.upload_retry_waiting);
    if expected_upload_settled > status.upload_planned_total {
        violations.push("upload_lane_overcommitted".to_string());
    }

    if matches!(status.phase.as_str(), "paused" | "idle" | "error")
        && status.current_activity.progress_mode != "hidden"
    {
        violations.push("lifecycle_progress_mode_invalid_for_phase".to_string());
    }

    if status.current_activity.stage.trim().is_empty() {
        violations.push("lifecycle_stage_missing".to_string());
    }

    let active_transfer_rows = status
        .in_progress
        .iter()
        .filter(|entry| entry.state == "in_progress")
        .count();
    let in_flight_counters = status
        .remote_download_in_flight
        .saturating_add(status.upload_in_flight);
    if active_transfer_rows == 0 && in_flight_counters > 0 {
        violations.push("in_flight_counter_without_active_rows".to_string());
    }

    if status.phase == "applying_remote"
        && status.planner_need_download_total > 0
        && status.remote_download_planned_total == 0
    {
        violations.push("planner_download_actions_without_materialized_jobs".to_string());
    }

    if status.phase == "applying_local"
        && status.planner_need_upload_total > 0
        && status.upload_planned_total == 0
    {
        violations.push("planner_upload_actions_without_materialized_jobs".to_string());
    }

    violations
}

pub(crate) fn ensure_auth_required_issue_contract(status: &mut SyncRuntimeAccountStatus) {
    if status.auth_ready {
        return;
    }
    if status.issue_code.is_none() {
        status.issue_code = Some("auth_required".to_string());
    }
    if status
        .issue_message
        .as_deref()
        .map(str::trim)
        .is_none_or(|value| value.is_empty())
    {
        status.issue_message =
            Some("Authentication required. Re-authenticate to resume synchronization.".to_string());
    }
    ensure_issue_action(status, "reauthenticate");
    ensure_issue_action(status, "retry_sync");
}

pub(crate) fn ensure_issue_action(status: &mut SyncRuntimeAccountStatus, action: &str) {
    if !status
        .issue_actions
        .iter()
        .any(|existing| existing == action)
    {
        status.issue_actions.push(action.to_string());
    }
}

pub(crate) fn issue_severity_from_code(issue_code: Option<&str>) -> &'static str {
    let Some(code) = issue_code else {
        return "none";
    };
    match code {
        "auth_required"
        | "permission_denied"
        | "disk_full"
        | "sync_root_unavailable"
        | "large_delete_guard"
        | "unknown_error" => "blocking",
        _ => "warning",
    }
}
