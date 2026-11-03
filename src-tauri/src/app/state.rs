use crate::app::sync_runtime::SyncRuntimeMap;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

pub struct DeviceAuthPending {
    pub profile_id: String,
    pub device_code: String,
    pub interval_secs: u64,
}

pub struct InteractiveAuthPending {
    pub profile_id: String,
    pub state: String,
    pub window_label: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub token_url: String,
}

pub struct AppState {
    pub profiles_lock: Arc<Mutex<()>>,
    pub auth_pending: Arc<Mutex<HashMap<String, DeviceAuthPending>>>,
    pub interactive_auth_pending: Arc<Mutex<HashMap<String, InteractiveAuthPending>>>,
    pub sync_worker_stops: Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>>,
    /// Each account's sync worker task, so an update can wait for it to exit.
    pub sync_worker_tasks: Arc<Mutex<HashMap<String, tauri::async_runtime::JoinHandle<()>>>>,
    pub sync_runtime: Arc<Mutex<SyncRuntimeMap>>,
    pub sync_cancel_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    pub sync_cycle_locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

pub fn create_app_state() -> AppState {
    AppState {
        profiles_lock: Arc::new(Mutex::new(())),
        auth_pending: Arc::new(Mutex::new(HashMap::new())),
        interactive_auth_pending: Arc::new(Mutex::new(HashMap::new())),
        sync_worker_stops: Arc::new(Mutex::new(HashMap::new())),
        sync_worker_tasks: Arc::new(Mutex::new(HashMap::new())),
        sync_runtime: Arc::new(Mutex::new(HashMap::new())),
        sync_cancel_flags: Arc::new(Mutex::new(HashMap::new())),
        sync_cycle_locks: Arc::new(Mutex::new(HashMap::new())),
    }
}
