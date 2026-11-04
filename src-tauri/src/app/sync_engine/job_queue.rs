use super::*;

pub(crate) const DOWNLOAD_JOB_DIRECTION: &str = "download";
pub(crate) const UPLOAD_JOB_DIRECTION: &str = "upload";
pub(crate) const DOWNLOAD_JOB_STATE_QUEUED: &str = "queued";
pub(crate) const DOWNLOAD_JOB_STATE_IN_PROGRESS: &str = "in_progress";
pub(crate) const DOWNLOAD_JOB_STATE_RETRY_WAIT: &str = "retry_wait";
pub(crate) const DOWNLOAD_JOB_STATE_DONE: &str = "done";
pub(crate) const DOWNLOAD_JOB_STATE_FAILED_TERMINAL: &str = "failed_terminal";
pub(crate) const DOWNLOAD_JOB_STATE_SKIPPED: &str = "skipped";
pub(crate) const JOB_RUN_STATE_IDLE: &str = "idle";
pub(crate) const JOB_RUN_STATE_CLAIMED: &str = "claimed";
pub(crate) const JOB_RUN_STATE_RUNNING: &str = "running";
pub(crate) const DOWNLOAD_JOB_LEASE_SECONDS: i64 = 900;
pub(crate) const ACTION_JOB_LEASE_SECONDS: i64 = 900;

#[derive(Debug, Clone)]
pub(crate) struct ClaimedDownloadJob {
    pub(crate) job_id: i64,
    pub(crate) item_id: String,
    pub(crate) path: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DownloadJobCounters {
    pub(crate) planned_total: usize,
    pub(crate) planned_bytes: u64,
    pub(crate) in_progress: usize,
    pub(crate) in_flight_bytes_done: u64,
    pub(crate) retry_waiting: usize,
    pub(crate) completed: usize,
    pub(crate) completed_bytes: u64,
    pub(crate) failed_terminal: usize,
    pub(crate) remaining: usize,
    pub(crate) remaining_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct UploadJobCounters {
    pub(crate) planned_total: usize,
    pub(crate) planned_bytes: u64,
    pub(crate) in_progress: usize,
    pub(crate) in_flight_bytes_done: u64,
    pub(crate) retry_waiting: usize,
    pub(crate) completed: usize,
    pub(crate) completed_bytes: u64,
    pub(crate) failed_terminal: usize,
    pub(crate) remaining_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SyncJobActivityProjection {
    pub(crate) active: Vec<SyncRuntimeTransfer>,
    pub(crate) recent_completed: Vec<SyncRuntimeRecentItem>,
    pub(crate) recent_retry_waiting: Vec<SyncRuntimeRecentItem>,
    pub(crate) recent_failed: Vec<SyncRuntimeRecentItem>,
    pub(crate) active_download_count: usize,
    pub(crate) active_upload_count: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SyncFilePlannerCounters {
    pub(crate) cloud_discovered_total: usize,
    pub(crate) local_discovered_total: usize,
    pub(crate) none_total: usize,
    pub(crate) need_download_total: usize,
    pub(crate) need_upload_total: usize,
    pub(crate) need_delete_remote_total: usize,
    pub(crate) need_delete_local_total: usize,
    pub(crate) conflict_total: usize,
    pub(crate) shared_reference_total: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct PersistedSyncIssue {
    pub(crate) issue_code: String,
    pub(crate) issue_message: String,
    pub(crate) issue_actions: Vec<String>,
    pub(crate) issue_path: Option<String>,
    pub(crate) issue_secondary_path: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ThrottleCounters {
    pub(crate) download_total: usize,
    pub(crate) download_last_minute: usize,
    pub(crate) upload_total: usize,
    pub(crate) upload_last_minute: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct SyncLifecycleStateRow {
    pub(crate) delta_link: Option<String>,
    pub(crate) active_delta_next_link: Option<String>,
    pub(crate) last_cycle_at: Option<String>,
    pub(crate) phase: String,
    pub(crate) phase_message: String,
    pub(crate) remote_scan_complete: bool,
    pub(crate) activity_stage: String,
    pub(crate) activity_progress_mode: String,
    pub(crate) activity_current: Option<usize>,
    pub(crate) activity_total: Option<usize>,
    pub(crate) activity_unit: Option<String>,
    pub(crate) activity_detail: Option<String>,
    pub(crate) activity_cycle_id: Option<String>,
    pub(crate) activity_updated_at: i64,
    pub(crate) large_delete_guard_approved: bool,
    pub(crate) large_delete_pending_paths_json: String,
    pub(crate) agent_state: String,
    pub(crate) last_sync_at: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LargeDeleteGuardState {
    pub(crate) approved: bool,
    pub(crate) pending_paths: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SyncLifecycleOperationalState {
    pub(crate) delta_link: Option<String>,
    pub(crate) active_delta_next_link: Option<String>,
    pub(crate) last_cycle_at: Option<String>,
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
