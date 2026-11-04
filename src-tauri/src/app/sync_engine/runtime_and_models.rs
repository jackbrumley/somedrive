use super::*;

#[derive(Default)]
pub(crate) struct SyncCycleStats {
    pub(crate) account_prefix: String,
    pub(crate) cycle_id: String,
    pub(crate) downloaded_files: usize,
    pub(crate) uploaded_files: usize,
    pub(crate) upload_failures: usize,
    pub(crate) baseline_refreshes: usize,
    pub(crate) matched_files: usize,
    pub(crate) conflict_copies: usize,
    pub(crate) conflicts_held: usize,
    pub(crate) local_moves: usize,
    pub(crate) deleted_local: usize,
    pub(crate) deleted_remote: usize,
    pub(crate) created_local_folders: usize,
    pub(crate) created_remote_folders: usize,
    pub(crate) remote_pages: usize,
    pub(crate) remote_items_received: usize,
    pub(crate) remote_items_skipped_unresolved: usize,
    pub(crate) local_items_seen: usize,
    pub(crate) name_issues: usize,
}

pub(crate) enum RemoteDownloadOutcome {
    /// The file is in place; `entry` is its state on disk afterwards.
    Downloaded(LocalEntry),
    /// The local file changed after the download was planned, so it was not
    /// replaced: the path is held as a conflict for the user.
    HeldAsConflict,
    /// Graph returned 404/410: the item may be gone. Nothing changes until a
    /// delta page confirms it; the job retries later.
    NotFoundOnCloud,
}

pub(crate) struct DeltaPageWorkItem {
    pub(crate) items: Vec<GraphDriveItem>,
    pub(crate) next_link: Option<String>,
    pub(crate) delta_link: Option<String>,
}

pub(crate) struct RemoteDownloadJob {
    /// A still-fresh direct content URL from the change feed, if any.
    pub(crate) direct_url: Option<String>,
    pub(crate) job_id: i64,
    pub(crate) item_id: String,
    pub(crate) path: String,
    pub(crate) local_abs: PathBuf,
    /// The cloud version planned for download.
    pub(crate) version: CloudVersion,
    pub(crate) cloud_mtime: i64,
}

pub(crate) enum RemoteDownloadResultStatus {
    Success(RemoteDownloadOutcome),
    DeferredRetry,
    Cancelled,
    Failed(String),
}

pub(crate) struct RemoteDownloadResult {
    pub(crate) job_id: i64,
    pub(crate) item_id: String,
    pub(crate) path: String,
    pub(crate) version: CloudVersion,
    pub(crate) status: RemoteDownloadResultStatus,
}

pub(crate) fn resolve_download_concurrency() -> usize {
    std::env::var("SOMEDRIVE_SYNC_DOWNLOAD_CONCURRENCY")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .map(crate::app::app_settings::clamp_download_concurrency)
        .or_else(|| {
            crate::app::app_settings::load_sync_download_concurrency()
                .ok()
                .map(|setting| setting.value)
        })
        .unwrap_or(DEFAULT_DOWNLOAD_CONCURRENCY)
}

pub(crate) fn resolve_delta_page_queue_capacity() -> usize {
    std::env::var("SOMEDRIVE_SYNC_DELTA_QUEUE_CAPACITY")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .map(|value| value.clamp(1, 128))
        .unwrap_or(8)
}

pub(crate) fn resolve_download_queue_capacity() -> usize {
    std::env::var("SOMEDRIVE_SYNC_DOWNLOAD_QUEUE_CAPACITY")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .map(|value| value.clamp(16, 4096))
        .unwrap_or(512)
}

pub(crate) fn resolve_simple_upload_max_bytes() -> u64 {
    std::env::var("SOMEDRIVE_SYNC_SIMPLE_UPLOAD_MAX_BYTES")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|value| value.clamp(1, 64 * 1024 * 1024))
        .unwrap_or(4 * 1024 * 1024)
}

pub(crate) fn resolve_upload_chunk_bytes() -> usize {
    const CHUNK_GRANULARITY: usize = 320 * 1024;
    std::env::var("SOMEDRIVE_SYNC_UPLOAD_CHUNK_BYTES")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .map(|value| value.clamp(CHUNK_GRANULARITY, 60 * 1024 * 1024))
        .map(|value| value - (value % CHUNK_GRANULARITY))
        .filter(|value| *value >= CHUNK_GRANULARITY)
        .unwrap_or(8 * 1024 * 1024)
}

pub(crate) fn resolve_request_timeout() -> Duration {
    std::env::var("SOMEDRIVE_SYNC_REQUEST_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|seconds| seconds.clamp(5, 300))
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECONDS))
}

pub(crate) fn resolve_connect_timeout() -> Duration {
    std::env::var("SOMEDRIVE_SYNC_CONNECT_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|seconds| seconds.clamp(1, 120))
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECONDS))
}

pub(crate) fn resolve_download_timeout_base() -> Duration {
    std::env::var("SOMEDRIVE_SYNC_DOWNLOAD_TIMEOUT_BASE_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|seconds| seconds.clamp(5, 600))
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(DEFAULT_DOWNLOAD_TIMEOUT_BASE_SECONDS))
}

pub(crate) fn resolve_download_timeout_min() -> Duration {
    std::env::var("SOMEDRIVE_SYNC_DOWNLOAD_TIMEOUT_MIN_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|seconds| seconds.clamp(10, 1800))
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(DEFAULT_DOWNLOAD_TIMEOUT_MIN_SECONDS))
}

pub(crate) fn resolve_download_timeout_max() -> Duration {
    std::env::var("SOMEDRIVE_SYNC_DOWNLOAD_TIMEOUT_MAX_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|seconds| seconds.clamp(30, 7200))
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(DEFAULT_DOWNLOAD_TIMEOUT_MAX_SECONDS))
}

pub(crate) fn resolve_download_timeout_per_mib_millis() -> u64 {
    std::env::var("SOMEDRIVE_SYNC_DOWNLOAD_TIMEOUT_PER_MIB_MILLIS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|millis| millis.clamp(100, 20_000))
        .unwrap_or(DEFAULT_DOWNLOAD_TIMEOUT_PER_MIB_MILLIS)
}

pub(crate) fn compute_download_request_timeout(known_size_bytes: Option<u64>) -> Duration {
    let base = resolve_download_timeout_base();
    let fallback = resolve_request_timeout().max(resolve_download_timeout_min());
    let max_timeout = resolve_download_timeout_max();

    let mut timeout = if let Some(size_bytes) = known_size_bytes.filter(|size| *size > 0) {
        let mib = size_bytes as f64 / (1024.0 * 1024.0);
        let per_mib_millis = resolve_download_timeout_per_mib_millis() as f64;
        let scaled_millis = (mib * per_mib_millis).round() as u64;
        base.saturating_add(Duration::from_millis(scaled_millis))
    } else {
        fallback
    };

    timeout = timeout.max(resolve_download_timeout_min());
    timeout.min(max_timeout)
}

pub(crate) fn resolve_stall_timeout() -> Duration {
    std::env::var("SOMEDRIVE_SYNC_STALL_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|seconds| seconds.clamp(10, 600))
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(DEFAULT_STALL_TIMEOUT_SECONDS))
}

pub(crate) fn resolve_large_delete_guard_threshold() -> usize {
    std::env::var("SOMEDRIVE_SYNC_LARGE_DELETE_THRESHOLD")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .map(|value| value.clamp(1, 100_000))
        .unwrap_or(10)
}

pub(crate) fn current_unix_seconds() -> i64 {
    chrono::Utc::now().timestamp()
}

pub(crate) fn graph_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(resolve_connect_timeout())
        .timeout(resolve_request_timeout())
        .build()
        .map_err(|error| format!("Failed creating Graph HTTP client: {error}"))
}

pub(crate) fn parse_retry_after_delay(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let retry_after = headers.get(reqwest::header::RETRY_AFTER)?;
    let text = retry_after.to_str().ok()?.trim();
    if text.is_empty() {
        return None;
    }
    let seconds = text.parse::<u64>().ok()?;
    Some(Duration::from_secs(seconds.min(MAX_RETRY_DELAY_SECONDS)))
}

pub(crate) fn parse_retry_after_seconds_from_json_body(body: &str) -> Option<Duration> {
    if body.trim().is_empty() {
        return None;
    }
    let parsed = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let value = parsed
        .get("error")
        .and_then(|error| {
            error
                .get("retryAfterSeconds")
                .or_else(|| error.get("retry_after_seconds"))
        })
        .or_else(|| {
            parsed
                .get("retryAfterSeconds")
                .or_else(|| parsed.get("retry_after_seconds"))
        })?;

    let seconds = if let Some(number) = value.as_u64() {
        number
    } else {
        let text = value.as_str()?;
        text.trim().parse::<u64>().ok()?
    };
    Some(Duration::from_secs(seconds.clamp(1, 3600)))
}

pub(crate) fn exponential_backoff_delay(attempt: u32) -> Duration {
    let exponent = attempt.saturating_sub(1).min(6);
    let seconds = 2_u64.saturating_pow(exponent).min(MAX_RETRY_DELAY_SECONDS);
    Duration::from_secs(seconds)
}

pub(crate) fn is_sync_cancelled_error(error: &str) -> bool {
    error == SYNC_CANCELLED_ERROR
}

pub(crate) fn ensure_not_cancelled(cancel_flag: &Arc<AtomicBool>) -> Result<(), String> {
    if cancel_flag.load(Ordering::Relaxed) {
        return Err(SYNC_CANCELLED_ERROR.to_string());
    }
    Ok(())
}

pub(crate) fn set_cancel_flag(
    state: &tauri::State<'_, AppState>,
    profile_id: &str,
    value: bool,
) -> Result<Arc<AtomicBool>, String> {
    let mut flags = state
        .sync_cancel_flags
        .lock()
        .map_err(|_| "Sync cancel flag lock is poisoned".to_string())?;
    let flag = flags
        .entry(profile_id.to_string())
        .or_insert_with(|| Arc::new(AtomicBool::new(false)))
        .clone();
    flag.store(value, Ordering::Relaxed);
    Ok(flag)
}

pub(crate) fn get_or_create_cycle_lock(
    state: &tauri::State<'_, AppState>,
    profile_id: &str,
) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
    let mut locks = state
        .sync_cycle_locks
        .lock()
        .map_err(|_| "Sync cycle lock map is poisoned".to_string())?;
    Ok(locks
        .entry(profile_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone())
}

pub(crate) async fn wait_for_cancellation(cancel_flag: Arc<AtomicBool>) {
    while !cancel_flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(CANCEL_POLL_INTERVAL_MILLIS)).await;
    }
}

pub(crate) async fn sleep_with_cancellation(
    cancel_flag: &Arc<AtomicBool>,
    duration: Duration,
) -> Result<(), String> {
    ensure_not_cancelled(cancel_flag)?;
    if duration.is_zero() {
        return Ok(());
    }
    tokio::select! {
        _ = wait_for_cancellation(Arc::clone(cancel_flag)) => Err(SYNC_CANCELLED_ERROR.to_string()),
        _ = tokio::time::sleep(duration) => Ok(()),
    }
}

pub(crate) fn build_unique_download_temp_path(
    local_path: &Path,
    item_id: &str,
    relative_path: &str,
    cycle_id: &str,
) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(item_id, &mut hasher);
    std::hash::Hash::hash(relative_path, &mut hasher);
    std::hash::Hash::hash(cycle_id, &mut hasher);
    let suffix = format!("{:016x}", std::hash::Hasher::finish(&hasher));
    let parent = local_path.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!(".somedrive-part-{suffix}.tmp"))
}

// the retry decision needs the job, transfer and cycle identifiers explicitly; a cycle context struct is tracked in docs/TODO.md.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_download_retry(
    graph: &GraphContext,
    job_id: Option<i64>,
    cancel_flag: &Arc<AtomicBool>,
    transfer_id: &Option<String>,
    attempt: u32,
    relative_path: &str,
    reason: &str,
    temp_path: Option<&Path>,
    final_error: &str,
) -> Result<bool, String> {
    if let Some(path) = temp_path {
        let _ = std::fs::remove_file(path);
    }
    if attempt < MAX_DOWNLOAD_RETRIES {
        let delay = exponential_backoff_delay(attempt);
        if let Some(active_job_id) = job_id {
            if let Err(error) =
                record_download_job_retrying_in_place(&graph.profile_id, active_job_id, reason)
            {
                log::warn!(
                    "{} [cycle:{}] DOWNLOAD_RETRY_PERSIST_FAILED path={} error={}",
                    graph.account_prefix,
                    graph.cycle_id,
                    relative_path,
                    error
                );
            }
        }
        log::warn!(
            "{} [cycle:{}] DOWNLOAD_RETRY attempt={} path={} reason={} delay_ms={}",
            graph.account_prefix,
            graph.cycle_id,
            attempt,
            relative_path,
            reason,
            delay.as_millis()
        );
        sleep_with_cancellation(cancel_flag, delay).await?;
        return Ok(true);
    }
    if let Some(active_transfer_id) = transfer_id {
        runtime_finish_transfer_error(
            &graph.sync_runtime,
            &graph.profile_id,
            active_transfer_id,
            final_error,
        );
    }
    Ok(false)
}

pub(crate) fn runtime_start_transfer(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    direction: &str,
    path: &str,
    bytes_total: Option<u64>,
) -> Option<String> {
    let mut runtime_map = runtime.lock().ok()?;
    Some(sync_runtime::start_transfer(
        &mut runtime_map,
        profile_id,
        direction,
        path,
        bytes_total,
    ))
}

pub(crate) fn runtime_update_transfer_progress(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    transfer_id: &str,
    bytes_done: u64,
    bytes_total: Option<u64>,
) {
    if let Ok(mut runtime_map) = runtime.lock() {
        sync_runtime::update_transfer_progress(
            &mut runtime_map,
            profile_id,
            transfer_id,
            bytes_done,
            bytes_total,
        );
    }
}

pub(crate) fn runtime_finish_transfer_success(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    transfer_id: &str,
) {
    if let Ok(mut runtime_map) = runtime.lock() {
        sync_runtime::finish_transfer_success(&mut runtime_map, profile_id, transfer_id);
    }
}

pub(crate) fn runtime_finish_transfer_error(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    transfer_id: &str,
    error: &str,
) {
    if let Ok(mut runtime_map) = runtime.lock() {
        sync_runtime::finish_transfer_error(&mut runtime_map, profile_id, transfer_id, error);
    }
}

pub(crate) fn runtime_finish_transfer_cancelled(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    transfer_id: &Option<String>,
) {
    if let Some(active_transfer_id) = transfer_id {
        runtime_finish_transfer_error(
            runtime,
            profile_id,
            active_transfer_id,
            SYNC_CANCELLED_ERROR,
        );
    }
}

pub(crate) fn runtime_finish_transfer_download_success(
    runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    transfer_id: &Option<String>,
    downloaded_bytes: u64,
) {
    if let Some(active_transfer_id) = transfer_id {
        runtime_update_transfer_progress(
            runtime,
            profile_id,
            active_transfer_id,
            downloaded_bytes,
            Some(downloaded_bytes),
        );
        runtime_finish_transfer_success(runtime, profile_id, active_transfer_id);
    }
}

/// A file or folder found by the local scan. Times are nanoseconds since the
/// Unix epoch so a same-size edit within one second is still a change.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LocalEntry {
    pub(crate) is_dir: bool,
    pub(crate) size: u64,
    pub(crate) mtime_ns: i64,
}

/// The cloud state of one item, as stored in `sync_files` (cloud columns).
#[derive(Debug, Clone)]
pub(crate) struct CloudItem {
    pub(crate) id: String,
    pub(crate) parent_id: Option<String>,
    pub(crate) path: String,
    pub(crate) is_dir: bool,
    pub(crate) size: u64,
    pub(crate) mtime: i64,
    pub(crate) etag: Option<String>,
    pub(crate) ctag: Option<String>,
    pub(crate) quick_xor_hash: Option<String>,
    pub(crate) download_url: Option<String>,
    pub(crate) is_shared_reference: bool,
    pub(crate) shared_drive_id: Option<String>,
    pub(crate) shared_item_id: Option<String>,
    pub(crate) shared_kind: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DeltaResponse {
    pub(crate) value: Vec<GraphDriveItem>,
    #[serde(rename = "@odata.nextLink")]
    pub(crate) next_link: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    pub(crate) delta_link: Option<String>,
}

/// A Graph driveItem, as returned by delta pages, uploads and folder creates.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphDriveItem {
    pub(crate) id: String,
    pub(crate) name: Option<String>,
    pub(crate) size: Option<u64>,
    pub(crate) folder: Option<serde_json::Value>,
    pub(crate) file: Option<GraphFileFacet>,
    pub(crate) root: Option<serde_json::Value>,
    pub(crate) package: Option<serde_json::Value>,
    pub(crate) malware: Option<serde_json::Value>,
    pub(crate) remote_item: Option<serde_json::Value>,
    pub(crate) deleted: Option<serde_json::Value>,
    pub(crate) parent_reference: Option<ParentReference>,
    pub(crate) last_modified_date_time: Option<String>,
    #[serde(rename = "eTag")]
    pub(crate) e_tag: Option<String>,
    #[serde(rename = "cTag")]
    pub(crate) c_tag: Option<String>,
    /// A short-lived (about an hour), pre-authenticated content URL.
    #[serde(rename = "@microsoft.graph.downloadUrl")]
    pub(crate) download_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphFileFacet {
    pub(crate) hashes: Option<GraphFileHashes>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphFileHashes {
    pub(crate) quick_xor_hash: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ParentReference {
    pub(crate) id: Option<String>,
    pub(crate) path: Option<String>,
}

impl GraphDriveItem {
    pub(crate) fn quick_xor_hash(&self) -> Option<String> {
        self.file
            .as_ref()
            .and_then(|file| file.hashes.as_ref())
            .and_then(|hashes| hashes.quick_xor_hash.clone())
    }

    pub(crate) fn parent_id(&self) -> Option<String> {
        self.parent_reference
            .as_ref()
            .and_then(|reference| reference.id.clone())
    }

    /// The cloud state of this item at `path`.
    pub(crate) fn to_cloud_item(&self, path: String) -> CloudItem {
        CloudItem {
            id: self.id.clone(),
            parent_id: self.parent_id(),
            path,
            is_dir: self.folder.is_some(),
            size: self.size.unwrap_or(0),
            mtime: parse_rfc3339_seconds(self.last_modified_date_time.as_deref()),
            etag: self.e_tag.clone(),
            ctag: self.c_tag.clone(),
            quick_xor_hash: self.quick_xor_hash(),
            download_url: self.download_url.clone(),
            is_shared_reference: self.remote_item.is_some(),
            shared_drive_id: shared_drive_id_from_delta_item(self),
            shared_item_id: shared_item_id_from_delta_item(self),
            shared_kind: shared_kind_from_delta_item(self),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UploadSessionResponse {
    pub(crate) upload_url: String,
}
