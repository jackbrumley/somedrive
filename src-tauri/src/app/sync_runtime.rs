use crate::app::sync_engine::build_authoritative_runtime_status;
use chrono::Local;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use tauri::{AppHandle, Emitter};

const RECENT_COMPLETED_LIMIT: usize = 120;
const RECENT_FAILED_LIMIT: usize = 120;
static SYNC_RUNTIME_REVISION: AtomicU64 = AtomicU64::new(0);
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRuntimeTransfer {
    pub id: String,
    pub direction: String,
    pub path: String,
    pub state: String,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    pub started_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRuntimeRecentItem {
    pub id: String,
    pub direction: String,
    pub path: String,
    pub bytes_total: Option<u64>,
    pub finished_at: String,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRuntimeCurrentActivity {
    pub stage: String,
    pub progress_mode: String,
    pub current: Option<usize>,
    pub total: Option<usize>,
    pub unit: Option<String>,
    pub detail: Option<String>,
    pub cycle_id: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRuntimeConsistency {
    pub ok: bool,
    pub violations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRuntimeAccountStatus {
    pub profile_id: String,
    pub engine_state: String,
    pub phase: String,
    pub phase_message: String,
    pub issue_code: Option<String>,
    pub issue_message: Option<String>,
    pub issue_actions: Vec<String>,
    pub issue_path: Option<String>,
    pub issue_secondary_path: Option<String>,
    pub issue_severity: String,
    pub auth_ready: bool,
    pub can_sync: bool,
    pub in_progress: Vec<SyncRuntimeTransfer>,
    pub recent_completed: Vec<SyncRuntimeRecentItem>,
    pub recent_retry_waiting: Vec<SyncRuntimeRecentItem>,
    pub recent_failed: Vec<SyncRuntimeRecentItem>,
    pub planner_cloud_discovered_total: usize,
    pub planner_local_discovered_total: usize,
    pub planner_none_total: usize,
    pub planner_need_download_total: usize,
    pub planner_need_upload_total: usize,
    pub planner_need_delete_remote_total: usize,
    pub planner_need_delete_local_total: usize,
    pub planner_conflict_total: usize,
    pub remote_discovered_total: usize,
    pub remote_download_planned_total: usize,
    pub remote_download_completed_total: usize,
    pub remote_download_failed_total: usize,
    pub remote_download_in_flight: usize,
    pub remote_download_retry_waiting: usize,
    pub remote_download_planned_bytes_total: u64,
    pub remote_download_completed_bytes_total: u64,
    pub remote_download_remaining_bytes_total: u64,
    pub remote_download_in_flight_bytes_done: u64,
    pub remote_download_throttle_total: usize,
    pub remote_download_throttle_last_minute: usize,
    pub upload_planned_total: usize,
    pub upload_completed_total: usize,
    pub upload_failed_total: usize,
    pub upload_in_flight: usize,
    pub upload_retry_waiting: usize,
    pub upload_planned_bytes_total: u64,
    pub upload_completed_bytes_total: u64,
    pub upload_remaining_bytes_total: u64,
    pub upload_in_flight_bytes_done: u64,
    pub upload_throttle_total: usize,
    pub upload_throttle_last_minute: usize,
    pub remote_scan_complete: bool,
    pub local_scan_scanned_count: usize,
    pub local_scan_estimated_total: Option<usize>,
    pub local_scan_current_path: Option<String>,
    pub current_activity: SyncRuntimeCurrentActivity,
    pub consistency: SyncRuntimeConsistency,
    pub updated_at: String,
}

impl SyncRuntimeAccountStatus {
    fn new(profile_id: &str) -> Self {
        let now = now_rfc3339();
        Self {
            profile_id: profile_id.to_string(),
            engine_state: "paused".to_string(),
            phase: "idle".to_string(),
            phase_message: "Idle".to_string(),
            issue_code: None,
            issue_message: None,
            issue_actions: Vec::new(),
            issue_path: None,
            issue_secondary_path: None,
            issue_severity: "none".to_string(),
            auth_ready: false,
            can_sync: false,
            in_progress: Vec::new(),
            recent_completed: Vec::new(),
            recent_retry_waiting: Vec::new(),
            recent_failed: Vec::new(),
            planner_cloud_discovered_total: 0,
            planner_local_discovered_total: 0,
            planner_none_total: 0,
            planner_need_download_total: 0,
            planner_need_upload_total: 0,
            planner_need_delete_remote_total: 0,
            planner_need_delete_local_total: 0,
            planner_conflict_total: 0,
            remote_discovered_total: 0,
            remote_download_planned_total: 0,
            remote_download_completed_total: 0,
            remote_download_failed_total: 0,
            remote_download_in_flight: 0,
            remote_download_retry_waiting: 0,
            remote_download_planned_bytes_total: 0,
            remote_download_completed_bytes_total: 0,
            remote_download_remaining_bytes_total: 0,
            remote_download_in_flight_bytes_done: 0,
            remote_download_throttle_total: 0,
            remote_download_throttle_last_minute: 0,
            upload_planned_total: 0,
            upload_completed_total: 0,
            upload_failed_total: 0,
            upload_in_flight: 0,
            upload_retry_waiting: 0,
            upload_planned_bytes_total: 0,
            upload_completed_bytes_total: 0,
            upload_remaining_bytes_total: 0,
            upload_in_flight_bytes_done: 0,
            upload_throttle_total: 0,
            upload_throttle_last_minute: 0,
            remote_scan_complete: false,
            local_scan_scanned_count: 0,
            local_scan_estimated_total: None,
            local_scan_current_path: None,
            current_activity: SyncRuntimeCurrentActivity {
                stage: "idle".to_string(),
                progress_mode: "hidden".to_string(),
                current: None,
                total: None,
                unit: None,
                detail: Some("Idle".to_string()),
                cycle_id: None,
                updated_at: now.clone(),
            },
            consistency: SyncRuntimeConsistency {
                ok: true,
                violations: Vec::new(),
            },
            updated_at: now,
        }
    }

    pub fn canonical_seed(profile_id: &str, auth_ready: bool) -> Self {
        let mut status = Self::new(profile_id);
        status.auth_ready = auth_ready;
        status
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRuntimeSnapshot {
    pub generated_at: String,
    pub revision: u64,
    pub accounts: Vec<SyncRuntimeAccountStatus>,
}

pub type SyncRuntimeMap = HashMap<String, SyncRuntimeAccountStatus>;

pub fn snapshot(runtime_map: &SyncRuntimeMap) -> SyncRuntimeSnapshot {
    let mut accounts: Vec<SyncRuntimeAccountStatus> = runtime_map.values().cloned().collect();
    for status in &mut accounts {
        recompute_authority_fields(status);
    }
    accounts.sort_by(|left, right| left.profile_id.cmp(&right.profile_id));
    SyncRuntimeSnapshot {
        generated_at: now_rfc3339(),
        revision: current_runtime_revision(),
        accounts,
    }
}

pub fn set_auth_ready(runtime_map: &mut SyncRuntimeMap, profile_id: &str, auth_ready: bool) {
    let status = ensure_account_status(runtime_map, profile_id);
    let previous_auth_ready = status.auth_ready;
    if previous_auth_ready == auth_ready {
        return;
    }
    if auth_ready && status.issue_code.as_deref() == Some("auth_required") {
        status.issue_code = None;
        status.issue_message = None;
        status.issue_actions.clear();
        status.issue_path = None;
        status.issue_secondary_path = None;
        log::info!(
            "{} SYNC_AUTH_REQUIRED_ISSUE_CLEARED reason=auth_ready",
            profile_id
        );
    }
    status.auth_ready = auth_ready;
    status.updated_at = now_rfc3339();
    bump_runtime_revision();
    log::info!(
        "{} SYNC_AUTH_AUTHORITY_TRANSITION from={} to={}",
        profile_id,
        previous_auth_ready,
        auth_ready
    );
    request_status_event(profile_id, StatusEventEmitMode::Immediate);
}

pub fn set_engine_state(runtime_map: &mut SyncRuntimeMap, profile_id: &str, engine_state: &str) {
    let status = ensure_account_status(runtime_map, profile_id);
    status.engine_state = engine_state.to_string();
    status.updated_at = now_rfc3339();
    bump_runtime_revision();
    request_status_event(profile_id, StatusEventEmitMode::Immediate);
}

pub fn set_phase(
    runtime_map: &mut SyncRuntimeMap,
    profile_id: &str,
    phase: &str,
    phase_message: &str,
) {
    let now = now_rfc3339();
    let status = ensure_account_status(runtime_map, profile_id);
    status.phase = phase.to_string();
    status.phase_message = phase_message.to_string();
    status.current_activity.stage = phase.to_string();
    status.current_activity.progress_mode = match phase {
        "paused" | "idle" | "error" => "hidden".to_string(),
        _ => "indeterminate".to_string(),
    };
    status.current_activity.current = None;
    status.current_activity.total = None;
    status.current_activity.unit = None;
    status.current_activity.detail = Some(phase_message.to_string());
    status.current_activity.cycle_id = None;
    status.current_activity.updated_at = now.clone();
    if phase != "scanning_local" {
        status.local_scan_scanned_count = 0;
        status.local_scan_estimated_total = None;
        status.local_scan_current_path = None;
    }
    status.updated_at = now;
    bump_runtime_revision();
    request_status_event(profile_id, StatusEventEmitMode::Immediate);
}

pub fn set_local_scan_progress(
    runtime_map: &mut SyncRuntimeMap,
    profile_id: &str,
    scanned_count: usize,
    estimated_total: Option<usize>,
    current_path: Option<&str>,
    cycle_id: Option<&str>,
) {
    let now = now_rfc3339();
    let status = ensure_account_status(runtime_map, profile_id);
    status.local_scan_scanned_count = scanned_count;
    status.local_scan_estimated_total = estimated_total;
    status.local_scan_current_path = current_path.map(ToString::to_string);
    status.current_activity.stage = "scanning_local".to_string();
    status.current_activity.progress_mode = if estimated_total.is_some() {
        "determinate".to_string()
    } else {
        "indeterminate".to_string()
    };
    status.current_activity.current = Some(scanned_count);
    status.current_activity.total = estimated_total;
    status.current_activity.unit = Some("files".to_string());
    status.current_activity.detail = status.local_scan_current_path.clone();
    status.current_activity.cycle_id = cycle_id.map(ToString::to_string);
    status.current_activity.updated_at = now.clone();
    status.updated_at = now;
    bump_runtime_revision();
    request_status_event(profile_id, StatusEventEmitMode::Throttled);
}

// the structured activity contract (stage, progress_mode, current, total, unit, detail, cycle_id) is passed field by field; bundling it into one struct is tracked in docs/TODO.md.
#[allow(clippy::too_many_arguments)]
pub fn set_current_activity(
    runtime_map: &mut SyncRuntimeMap,
    profile_id: &str,
    stage: &str,
    progress_mode: &str,
    current: Option<usize>,
    total: Option<usize>,
    unit: Option<&str>,
    detail: Option<&str>,
    cycle_id: Option<&str>,
) {
    let now = now_rfc3339();
    let status = ensure_account_status(runtime_map, profile_id);
    status.current_activity.stage = stage.to_string();
    status.current_activity.progress_mode = progress_mode.to_string();
    status.current_activity.current = current;
    status.current_activity.total = total;
    status.current_activity.unit = unit.map(ToString::to_string);
    status.current_activity.detail = detail.map(ToString::to_string);
    status.current_activity.cycle_id = cycle_id.map(ToString::to_string);
    status.current_activity.updated_at = now.clone();
    status.updated_at = now;
    bump_runtime_revision();
    request_status_event(profile_id, StatusEventEmitMode::Throttled);
}

pub fn set_issue(
    runtime_map: &mut SyncRuntimeMap,
    profile_id: &str,
    issue_code: &str,
    issue_message: &str,
    issue_actions: &[&str],
    issue_path: Option<&str>,
    issue_secondary_path: Option<&str>,
) {
    let status = ensure_account_status(runtime_map, profile_id);
    status.issue_code = Some(issue_code.to_string());
    status.issue_message = Some(issue_message.to_string());
    status.issue_actions = issue_actions
        .iter()
        .map(|action| (*action).to_string())
        .collect();
    status.issue_path = issue_path.map(|value| value.to_string());
    status.issue_secondary_path = issue_secondary_path.map(|value| value.to_string());
    if issue_code == "auth_required" {
        status.auth_ready = false;
    }
    status.updated_at = now_rfc3339();
    bump_runtime_revision();
    request_status_event(profile_id, StatusEventEmitMode::Immediate);
}

pub fn clear_issue(runtime_map: &mut SyncRuntimeMap, profile_id: &str) {
    let status = ensure_account_status(runtime_map, profile_id);
    status.issue_code = None;
    status.issue_message = None;
    status.issue_actions.clear();
    status.issue_path = None;
    status.issue_secondary_path = None;
    status.updated_at = now_rfc3339();
    bump_runtime_revision();
    request_status_event(profile_id, StatusEventEmitMode::Immediate);
}

pub fn reset_transfer_activity(runtime_map: &mut SyncRuntimeMap, profile_id: &str) {
    let status = ensure_account_status(runtime_map, profile_id);
    status.in_progress.clear();
    reset_remote_session_progress(status);
    reset_upload_session_progress(status);
    status.updated_at = now_rfc3339();
    bump_runtime_revision();
    request_status_event(profile_id, StatusEventEmitMode::Immediate);
}

pub fn set_remote_scan_complete(
    runtime_map: &mut SyncRuntimeMap,
    profile_id: &str,
    complete: bool,
) {
    let status = ensure_account_status(runtime_map, profile_id);
    status.remote_scan_complete = complete;
    status.updated_at = now_rfc3339();
    bump_runtime_revision();
    request_status_event(profile_id, StatusEventEmitMode::Immediate);
}

pub fn remove_account(runtime_map: &mut SyncRuntimeMap, profile_id: &str) {
    runtime_map.remove(profile_id);
    bump_runtime_revision();
    forget_status_events(profile_id);
    emit_removed_status_event(profile_id);
}

fn ensure_account_status<'a>(
    runtime_map: &'a mut SyncRuntimeMap,
    profile_id: &str,
) -> &'a mut SyncRuntimeAccountStatus {
    runtime_map
        .entry(profile_id.to_string())
        .or_insert_with(|| SyncRuntimeAccountStatus::new(profile_id))
}

fn reset_remote_session_progress(status: &mut SyncRuntimeAccountStatus) {
    status.remote_discovered_total = 0;
    status.remote_download_planned_total = 0;
    status.remote_download_completed_total = 0;
    status.remote_download_failed_total = 0;
    status.remote_download_in_flight = 0;
    status.remote_download_retry_waiting = 0;
    status.remote_download_planned_bytes_total = 0;
    status.remote_download_completed_bytes_total = 0;
    status.remote_download_remaining_bytes_total = 0;
    status.remote_download_in_flight_bytes_done = 0;
    status.remote_download_throttle_total = 0;
    status.remote_download_throttle_last_minute = 0;
    status.remote_scan_complete = false;
}

fn reset_upload_session_progress(status: &mut SyncRuntimeAccountStatus) {
    status.upload_planned_total = 0;
    status.upload_completed_total = 0;
    status.upload_failed_total = 0;
    status.upload_in_flight = 0;
    status.upload_retry_waiting = 0;
    status.upload_planned_bytes_total = 0;
    status.upload_completed_bytes_total = 0;
    status.upload_remaining_bytes_total = 0;
    status.upload_in_flight_bytes_done = 0;
    status.upload_throttle_total = 0;
    status.upload_throttle_last_minute = 0;
}

fn now_rfc3339() -> String {
    Local::now().to_rfc3339()
}

fn new_runtime_id(prefix: &str) -> String {
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    format!("{}-{}", prefix, nanos)
}

fn bump_runtime_revision() {
    SYNC_RUNTIME_REVISION.fetch_add(1, Ordering::Relaxed);
}

fn current_runtime_revision() -> u64 {
    SYNC_RUNTIME_REVISION.load(Ordering::Relaxed)
}

mod authority;
mod status_events;
mod transfers;
pub use authority::*;
pub use status_events::*;
pub use transfers::*;

#[cfg(test)]
#[path = "sync_runtime_tests.rs"]
mod tests;
