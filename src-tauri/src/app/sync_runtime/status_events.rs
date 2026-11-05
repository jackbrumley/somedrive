use super::*;
use std::sync::Arc;
use std::time::{Duration, Instant};

// Sending account status to the UI as `sync-status` events: throttling,
// sequence numbers, and the app handle the events go through.

/// An immediate change (a transfer starting or finishing, a phase change)
/// reaches the UI within this long, coalesced with any others.
const IMMEDIATE_STATUS_INTERVAL: Duration = Duration::from_millis(250);
/// Progress-only changes reach the UI at most this often.
const THROTTLED_STATUS_INTERVAL: Duration = Duration::from_millis(1000);
/// How often the emitter thread looks for accounts due a refresh.
const STATUS_EMITTER_TICK: Duration = Duration::from_millis(100);

pub(crate) static SYNC_STATUS_EVENT_SEQUENCE: LazyLock<Mutex<HashMap<String, u64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Accounts waiting for a status refresh (`true` when one of the changes was
/// immediate) and when each was last sent.
#[derive(Default)]
struct StatusEmitterState {
    pending: HashMap<String, bool>,
    last_emitted_at: HashMap<String, Instant>,
}

static STATUS_EMITTER: LazyLock<Mutex<StatusEmitterState>> =
    LazyLock::new(|| Mutex::new(StatusEmitterState::default()));

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

/// Stores the app handle and starts the status emitter thread, which builds
/// each account's status from the database and sends it to the UI.
pub fn initialize_status_event_stream(
    app_handle: AppHandle,
    sync_runtime: Arc<Mutex<SyncRuntimeMap>>,
) {
    if let Ok(mut handle) = SYNC_STATUS_APP_HANDLE.lock() {
        *handle = Some(app_handle);
    }
    let spawned = std::thread::Builder::new()
        .name("sync-status-emitter".to_string())
        .spawn(move || run_status_emitter(&sync_runtime));
    if let Err(error) = spawned {
        log::error!("SYNC_STATUS_EMITTER_START_FAILED error={}", error);
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

/// Asks for `profile_id`'s status to be sent to the UI. Cheap and safe to
/// call while holding the runtime map: the status is rebuilt from the
/// database on the emitter thread, never on the caller's (a download's
/// progress update must not wait for database reads made for the UI).
pub(crate) fn request_status_event(profile_id: &str, emit_mode: StatusEventEmitMode) {
    let immediate = matches!(emit_mode, StatusEventEmitMode::Immediate);
    if let Ok(mut emitter) = STATUS_EMITTER.lock() {
        let pending = emitter
            .pending
            .entry(profile_id.to_string())
            .or_insert(false);
        *pending = *pending || immediate;
    }
}

/// Drops any pending refresh for a removed account.
pub(crate) fn forget_status_events(profile_id: &str) {
    if let Ok(mut emitter) = STATUS_EMITTER.lock() {
        emitter.pending.remove(profile_id);
        emitter.last_emitted_at.remove(profile_id);
    }
}

fn run_status_emitter(sync_runtime: &Arc<Mutex<SyncRuntimeMap>>) {
    let mut change_watcher = ChangeWatcher::default();
    let mut last_change_check = Instant::now();
    loop {
        std::thread::sleep(STATUS_EMITTER_TICK);
        if last_change_check.elapsed() >= CHANGE_CHECK_INTERVAL {
            last_change_check = Instant::now();
            change_watcher.check(sync_runtime);
        }
        for profile_id in take_due_status_events() {
            emit_authoritative_status(sync_runtime, &profile_id);
        }
    }
}

/// Accounts whose refresh is due now; the rest stay pending, so the last
/// progress update is always sent eventually.
fn take_due_status_events() -> Vec<String> {
    let now = Instant::now();
    let Ok(mut emitter) = STATUS_EMITTER.lock() else {
        return Vec::new();
    };
    let StatusEmitterState {
        pending,
        last_emitted_at,
    } = &mut *emitter;
    let mut due = Vec::new();
    pending.retain(|profile_id, immediate| {
        let interval = if *immediate {
            IMMEDIATE_STATUS_INTERVAL
        } else {
            THROTTLED_STATUS_INTERVAL
        };
        let ready = last_emitted_at
            .get(profile_id)
            .is_none_or(|sent_at| now.duration_since(*sent_at) >= interval);
        if ready {
            due.push(profile_id.clone());
        }
        !ready
    });
    for profile_id in &due {
        last_emitted_at.insert(profile_id.clone(), now);
    }
    due
}

/// Builds the account's status from the database (no lock held) and sends
/// it. If the projection fails, the in-memory status is sent marked
/// inconsistent instead.
fn emit_authoritative_status(sync_runtime: &Arc<Mutex<SyncRuntimeMap>>, profile_id: &str) {
    let auth_ready = match sync_runtime.lock() {
        Ok(runtime_map) => match runtime_map.get(profile_id) {
            Some(status) => status.auth_ready,
            None => return,
        },
        Err(_) => return,
    };
    match build_authoritative_runtime_status(profile_id, auth_ready) {
        Ok(authoritative) => emit_upsert_status_event(profile_id, &authoritative),
        Err(error) => {
            log::warn!(
                "{} SYNC_STATUS_CANONICAL_PROJECTION_FAILED error={}",
                profile_id,
                error
            );
            let Some(mut degraded) = sync_runtime
                .lock()
                .ok()
                .and_then(|runtime_map| runtime_map.get(profile_id).cloned())
            else {
                return;
            };
            recompute_authority_fields(&mut degraded);
            degraded.consistency.ok = false;
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

#[cfg(test)]
mod status_emitter_tests {
    use super::*;

    fn due_for(profile_id: &str) -> usize {
        take_due_status_events()
            .into_iter()
            .filter(|due| due == profile_id)
            .count()
    }

    #[test]
    fn requests_coalesce_and_a_throttled_one_waits_but_is_not_lost() {
        let profile_id = format!(
            "status-emitter-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        request_status_event(&profile_id, StatusEventEmitMode::Throttled);
        request_status_event(&profile_id, StatusEventEmitMode::Immediate);
        request_status_event(&profile_id, StatusEventEmitMode::Immediate);
        assert_eq!(due_for(&profile_id), 1, "three requests, one refresh");

        request_status_event(&profile_id, StatusEventEmitMode::Throttled);
        assert_eq!(due_for(&profile_id), 0, "too soon after the last refresh");
        let still_pending = STATUS_EMITTER
            .lock()
            .expect("emitter")
            .pending
            .contains_key(&profile_id);
        assert!(still_pending, "the progress update is kept for later");

        forget_status_events(&profile_id);
    }
}
