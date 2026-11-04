use super::*;

// Transfers in progress and the download/upload counters on an account's
// status.

pub fn start_transfer(
    runtime_map: &mut SyncRuntimeMap,
    profile_id: &str,
    direction: &str,
    path: &str,
    bytes_total: Option<u64>,
) -> String {
    let now = now_rfc3339();
    let transfer_id = new_runtime_id("xfer");
    let status = ensure_account_status(runtime_map, profile_id);
    status.in_progress.push(SyncRuntimeTransfer {
        id: transfer_id.clone(),
        direction: direction.to_string(),
        path: path.to_string(),
        state: "in_progress".to_string(),
        bytes_done: 0,
        bytes_total,
        started_at: now.clone(),
        updated_at: now,
    });
    if direction.eq_ignore_ascii_case("upload") {
        status.upload_planned_total += 1;
        sync_upload_in_flight(status);
    }
    status.updated_at = now_rfc3339();
    bump_runtime_revision();
    request_status_event(profile_id, StatusEventEmitMode::Immediate);
    transfer_id
}

pub fn update_transfer_progress(
    runtime_map: &mut SyncRuntimeMap,
    profile_id: &str,
    transfer_id: &str,
    bytes_done: u64,
    bytes_total: Option<u64>,
) {
    let status = ensure_account_status(runtime_map, profile_id);
    if let Some(transfer) = status
        .in_progress
        .iter_mut()
        .find(|entry| entry.id == transfer_id)
    {
        transfer.bytes_done = bytes_done;
        if bytes_total.is_some() {
            transfer.bytes_total = bytes_total;
        }
        transfer.updated_at = now_rfc3339();
    }
    status.updated_at = now_rfc3339();
    bump_runtime_revision();
    request_status_event(profile_id, StatusEventEmitMode::Throttled);
}

pub fn finish_transfer_success(
    runtime_map: &mut SyncRuntimeMap,
    profile_id: &str,
    transfer_id: &str,
) {
    if let Some(item) = remove_transfer(runtime_map, profile_id, transfer_id) {
        let status = ensure_account_status(runtime_map, profile_id);
        if item.direction.eq_ignore_ascii_case("upload") {
            status.upload_completed_total += 1;
        }
        status.recent_completed.insert(
            0,
            SyncRuntimeRecentItem {
                id: item.id,
                direction: item.direction,
                path: item.path,
                bytes_total: item.bytes_total,
                finished_at: now_rfc3339(),
                status: "completed".to_string(),
                error: None,
            },
        );
        if status.recent_completed.len() > RECENT_COMPLETED_LIMIT {
            status.recent_completed.truncate(RECENT_COMPLETED_LIMIT);
        }
        sync_upload_in_flight(status);
        status.updated_at = now_rfc3339();
        bump_runtime_revision();
        request_status_event(profile_id, StatusEventEmitMode::Immediate);
    }
}

pub fn finish_transfer_error(
    runtime_map: &mut SyncRuntimeMap,
    profile_id: &str,
    transfer_id: &str,
    error_message: &str,
) {
    if let Some(item) = remove_transfer(runtime_map, profile_id, transfer_id) {
        let status = ensure_account_status(runtime_map, profile_id);
        if item.direction.eq_ignore_ascii_case("upload") {
            status.upload_failed_total += 1;
        }
        status.recent_failed.insert(
            0,
            SyncRuntimeRecentItem {
                id: item.id,
                direction: item.direction,
                path: item.path,
                bytes_total: item.bytes_total,
                finished_at: now_rfc3339(),
                status: "failed".to_string(),
                error: Some(error_message.to_string()),
            },
        );
        if status.recent_failed.len() > RECENT_FAILED_LIMIT {
            status.recent_failed.truncate(RECENT_FAILED_LIMIT);
        }
        sync_upload_in_flight(status);
        status.updated_at = now_rfc3339();
        bump_runtime_revision();
        request_status_event(profile_id, StatusEventEmitMode::Immediate);
    }
}

pub(crate) fn remove_transfer(
    runtime_map: &mut SyncRuntimeMap,
    profile_id: &str,
    transfer_id: &str,
) -> Option<SyncRuntimeTransfer> {
    let status = ensure_account_status(runtime_map, profile_id);
    let index = status
        .in_progress
        .iter()
        .position(|entry| entry.id == transfer_id)?;
    Some(status.in_progress.remove(index))
}

pub(crate) fn sync_upload_in_flight(status: &mut SyncRuntimeAccountStatus) {
    status.upload_in_flight = status
        .in_progress
        .iter()
        .filter(|entry| entry.direction.eq_ignore_ascii_case("upload"))
        .count();
}

pub fn set_remote_download_counters(
    runtime_map: &mut SyncRuntimeMap,
    profile_id: &str,
    planned_total: usize,
    completed_total: usize,
    failed_total: usize,
    in_flight: usize,
    retry_waiting: usize,
) {
    let status = ensure_account_status(runtime_map, profile_id);
    let _ = (
        planned_total,
        completed_total,
        failed_total,
        in_flight,
        retry_waiting,
    );
    status.updated_at = now_rfc3339();
    bump_runtime_revision();
    request_status_event(profile_id, StatusEventEmitMode::Throttled);
}

pub fn set_upload_counters(
    runtime_map: &mut SyncRuntimeMap,
    profile_id: &str,
    planned_total: usize,
    completed_total: usize,
    failed_total: usize,
    in_flight: usize,
) {
    let status = ensure_account_status(runtime_map, profile_id);
    let _ = (planned_total, completed_total, failed_total, in_flight);
    status.updated_at = now_rfc3339();
    bump_runtime_revision();
    request_status_event(profile_id, StatusEventEmitMode::Throttled);
}

pub fn set_upload_planned_total(
    runtime_map: &mut SyncRuntimeMap,
    profile_id: &str,
    planned_total: usize,
) {
    let status = ensure_account_status(runtime_map, profile_id);
    let _ = planned_total;
    status.updated_at = now_rfc3339();
    bump_runtime_revision();
    request_status_event(profile_id, StatusEventEmitMode::Throttled);
}

pub fn record_remote_download_completed(
    runtime_map: &mut SyncRuntimeMap,
    profile_id: &str,
    item_id: &str,
) {
    let _ = item_id;
    let status = ensure_account_status(runtime_map, profile_id);
    status.updated_at = now_rfc3339();
    bump_runtime_revision();
    request_status_event(profile_id, StatusEventEmitMode::Throttled);
}

pub fn record_remote_download_failed(
    runtime_map: &mut SyncRuntimeMap,
    profile_id: &str,
    item_id: &str,
) {
    let _ = item_id;
    let status = ensure_account_status(runtime_map, profile_id);
    status.updated_at = now_rfc3339();
    bump_runtime_revision();
    request_status_event(profile_id, StatusEventEmitMode::Immediate);
}
