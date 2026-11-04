use crate::app::sync_runtime::{SyncRuntimeRecentItem, SyncRuntimeTransfer};

const DOWNLOAD_JOB_DIRECTION: &str = "download";
const UPLOAD_JOB_DIRECTION: &str = "upload";
const DOWNLOAD_JOB_STATE_QUEUED: &str = "queued";
const DOWNLOAD_JOB_STATE_IN_PROGRESS: &str = "in_progress";
const DOWNLOAD_JOB_STATE_RETRY_WAIT: &str = "retry_wait";
const DOWNLOAD_JOB_STATE_DONE: &str = "done";
const DOWNLOAD_JOB_STATE_FAILED_TERMINAL: &str = "failed_terminal";
const DOWNLOAD_JOB_STATE_SKIPPED: &str = "skipped";
const JOB_RUN_STATE_IDLE: &str = "idle";
const JOB_RUN_STATE_CLAIMED: &str = "claimed";
const JOB_RUN_STATE_RUNNING: &str = "running";
const DOWNLOAD_JOB_LEASE_SECONDS: i64 = 900;
const ACTION_JOB_LEASE_SECONDS: i64 = 900;

#[derive(Debug, Clone)]
struct ClaimedDownloadJob {
    job_id: i64,
    item_id: String,
    path: String,
}

#[derive(Debug, Clone, Default)]
struct DownloadJobCounters {
    planned_total: usize,
    planned_bytes: u64,
    in_progress: usize,
    in_flight_bytes_done: u64,
    retry_waiting: usize,
    completed: usize,
    completed_bytes: u64,
    failed_terminal: usize,
    remaining: usize,
    remaining_bytes: u64,
}

#[derive(Debug, Clone, Default)]
struct UploadJobCounters {
    planned_total: usize,
    planned_bytes: u64,
    in_progress: usize,
    in_flight_bytes_done: u64,
    retry_waiting: usize,
    completed: usize,
    completed_bytes: u64,
    failed_terminal: usize,
    remaining_bytes: u64,
}

#[derive(Debug, Clone, Default)]
struct SyncJobActivityProjection {
    active: Vec<SyncRuntimeTransfer>,
    recent_completed: Vec<SyncRuntimeRecentItem>,
    recent_retry_waiting: Vec<SyncRuntimeRecentItem>,
    recent_failed: Vec<SyncRuntimeRecentItem>,
    active_download_count: usize,
    active_upload_count: usize,
}

#[derive(Debug, Clone, Default)]
struct SyncFilePlannerCounters {
    cloud_discovered_total: usize,
    local_discovered_total: usize,
    none_total: usize,
    need_download_total: usize,
    need_upload_total: usize,
    need_delete_remote_total: usize,
    need_delete_local_total: usize,
    conflict_total: usize,
    shared_reference_total: usize,
}

#[derive(Debug, Clone)]
struct PersistedSyncIssue {
    issue_code: String,
    issue_message: String,
    issue_actions: Vec<String>,
    issue_path: Option<String>,
    issue_secondary_path: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct ThrottleCounters {
    download_total: usize,
    download_last_minute: usize,
    upload_total: usize,
    upload_last_minute: usize,
}

#[derive(Debug, Clone)]
struct SyncLifecycleStateRow {
    delta_link: Option<String>,
    active_delta_next_link: Option<String>,
    last_cycle_at: Option<String>,
    phase: String,
    phase_message: String,
    remote_scan_complete: bool,
    activity_stage: String,
    activity_progress_mode: String,
    activity_current: Option<usize>,
    activity_total: Option<usize>,
    activity_unit: Option<String>,
    activity_detail: Option<String>,
    activity_cycle_id: Option<String>,
    activity_updated_at: i64,
    large_delete_guard_approved: bool,
    large_delete_pending_paths_json: String,
    agent_state: String,
    last_sync_at: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct LargeDeleteGuardState {
    approved: bool,
    pending_paths: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct SyncLifecycleOperationalState {
    delta_link: Option<String>,
    active_delta_next_link: Option<String>,
    last_cycle_at: Option<String>,
}

impl Default for SyncLifecycleStateRow {
    fn default() -> Self {
        Self {
            delta_link: None,
            active_delta_next_link: None,
            last_cycle_at: None,
            phase: "idle".to_string(),
            phase_message: "Idle".to_string(),
            remote_scan_complete: false,
            activity_stage: "idle".to_string(),
            activity_progress_mode: "hidden".to_string(),
            activity_current: None,
            activity_total: None,
            activity_unit: None,
            activity_detail: Some("Idle".to_string()),
            activity_cycle_id: None,
            activity_updated_at: 0,
            large_delete_guard_approved: false,
            large_delete_pending_paths_json: "[]".to_string(),
            agent_state: "idle".to_string(),
            last_sync_at: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryFailedDownloadJobStatus {
    Retried,
    AlreadyRetrying,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RetryAllFailedDownloadJobsReport {
    pub retried: usize,
    pub skipped_permission_denied: usize,
    pub already_retrying: usize,
}

include!("job_queue_db.rs");
include!("job_queue_lifecycle_store.rs");
include!("sync_files_store.rs");
include!("cloud_state_store.rs");
include!("sync_problems_store.rs");
include!("job_queue_download_store.rs");
include!("job_queue_upload_store.rs");
include!("job_queue_activity_projection.rs");
include!("job_queue_issue_throttle_store.rs");
include!("job_queue_tests.rs");
