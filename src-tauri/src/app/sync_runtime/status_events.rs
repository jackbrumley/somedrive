use super::*;

// Sending account status to the UI as `sync-status` events: throttling,
// sequence numbers, and the app handle the events go through.

pub(crate) const STATUS_EVENT_THROTTLE_MS: i64 = 1000;

pub(crate) static SYNC_STATUS_EVENT_SEQUENCE: LazyLock<Mutex<HashMap<String, u64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) static SYNC_STATUS_LAST_EMIT_MS_BY_ACCOUNT: LazyLock<Mutex<HashMap<String, i64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) static SYNC_STATUS_APP_HANDLE: LazyLock<Mutex<Option<AppHandle>>> =
    LazyLock::new(|| Mutex::new(None));

pub(crate) const SYNC_STATUS_EVENT_NAME: &str = "sync-status";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatusEvent {
    pub profile_id: String,
    pub status_seq: u64,
    pub generated_at: String,
    pub kind: String,
    pub status: Option<SyncRuntimeAccountStatus>,
}

pub fn initialize_status_event_stream(app_handle: AppHandle) {
    if let Ok(mut handle) = SYNC_STATUS_APP_HANDLE.lock() {
        *handle = Some(app_handle);
    }
}

pub fn emit_sync_status_snapshot_accounts(accounts: &[SyncRuntimeAccountStatus]) {
    let mut sorted_accounts: Vec<SyncRuntimeAccountStatus> = accounts.to_vec();
    sorted_accounts.sort_by(|left, right| left.profile_id.cmp(&right.profile_id));
    for mut account in sorted_accounts {
        recompute_authority_fields(&mut account);
        emit_upsert_status_event(&account.profile_id, &account);
    }
}

pub(crate) enum StatusEventEmitMode {
    Immediate,
    Throttled,
}

pub(crate) fn emit_status_event_for_account(
    runtime_map: &SyncRuntimeMap,
    profile_id: &str,
    emit_mode: StatusEventEmitMode,
) {
    let should_emit_now = match emit_mode {
        StatusEventEmitMode::Immediate => true,
        StatusEventEmitMode::Throttled => {
            let now_ms = chrono::Utc::now().timestamp_millis();
            if let Ok(mut last_emit_map) = SYNC_STATUS_LAST_EMIT_MS_BY_ACCOUNT.lock() {
                let last_emit_ms = *last_emit_map.get(profile_id).unwrap_or(&0);
                if now_ms - last_emit_ms < STATUS_EVENT_THROTTLE_MS {
                    false
                } else {
                    last_emit_map.insert(profile_id.to_string(), now_ms);
                    true
                }
            } else {
                true
            }
        }
    };
    if !should_emit_now {
        return;
    }

    if let Ok(mut last_emit_map) = SYNC_STATUS_LAST_EMIT_MS_BY_ACCOUNT.lock() {
        last_emit_map.insert(
            profile_id.to_string(),
            chrono::Utc::now().timestamp_millis(),
        );
    }
    if let Some(status) = runtime_map.get(profile_id) {
        match build_authoritative_runtime_status(profile_id, status.auth_ready) {
            Ok(authoritative) => emit_upsert_status_event(profile_id, &authoritative),
            Err(error) => {
                log::warn!(
                    "{} SYNC_STATUS_CANONICAL_PROJECTION_FAILED error={}",
                    profile_id,
                    error
                );
                let mut degraded = status.clone();
                recompute_authority_fields(&mut degraded);
                if degraded.consistency.ok {
                    degraded.consistency.ok = false;
                }
                if !degraded
                    .consistency
                    .violations
                    .iter()
                    .any(|entry| entry == "canonical_projection_failed")
                {
                    degraded
                        .consistency
                        .violations
                        .push("canonical_projection_failed".to_string());
                }
                emit_upsert_status_event(profile_id, &degraded);
            }
        }
    }
}

pub(crate) fn next_status_seq(profile_id: &str) -> u64 {
    if let Ok(mut sequence_map) = SYNC_STATUS_EVENT_SEQUENCE.lock() {
        let entry = sequence_map.entry(profile_id.to_string()).or_insert(0);
        *entry = entry.saturating_add(1);
        return *entry;
    }
    0
}

pub(crate) fn emit_upsert_status_event(profile_id: &str, status: &SyncRuntimeAccountStatus) {
    let status_seq = next_status_seq(profile_id);
    let payload = SyncStatusEvent {
        profile_id: profile_id.to_string(),
        status_seq,
        generated_at: now_rfc3339(),
        kind: "upsert".to_string(),
        status: Some(status.clone()),
    };
    emit_sync_status_payload(payload);
}

pub(crate) fn emit_removed_status_event(profile_id: &str) {
    let status_seq = next_status_seq(profile_id);
    let payload = SyncStatusEvent {
        profile_id: profile_id.to_string(),
        status_seq,
        generated_at: now_rfc3339(),
        kind: "removed".to_string(),
        status: None,
    };
    emit_sync_status_payload(payload);
}

pub(crate) fn emit_sync_status_payload(payload: SyncStatusEvent) {
    let app_handle = SYNC_STATUS_APP_HANDLE
        .lock()
        .ok()
        .and_then(|handle| (*handle).clone());
    if let Some(app_handle) = app_handle {
        if let Err(error) = app_handle.emit(SYNC_STATUS_EVENT_NAME, payload) {
            log::warn!("SYNC_STATUS_EVENT_EMIT_FAILED error={}", error);
        }
    }
}
