use super::*;

/// Per-cycle connection to Microsoft Graph for one account: the access token
/// (refreshed on demand), the account-wide throttle window, and the single
/// path that records a sign-in Microsoft no longer accepts.
#[derive(Clone)]
pub(crate) struct GraphContext {
    pub(crate) profile_id: String,
    pub(crate) account_prefix: String,
    pub(crate) cycle_id: String,
    pub(crate) access_token: Arc<tokio::sync::RwLock<String>>,
    pub(crate) token_refresh_lock: Arc<tokio::sync::Mutex<()>>,
    pub(crate) profiles_lock: Arc<std::sync::Mutex<()>>,
    pub(crate) sync_runtime: Arc<std::sync::Mutex<SyncRuntimeMap>>,
    /// Account-wide throttle window: after Microsoft answers 429, no Graph
    /// request for this account starts before this instant, so parallel
    /// workers back off together instead of each hitting the limit again.
    pub(crate) throttled_until: Arc<std::sync::Mutex<Option<tokio::time::Instant>>>,
    /// The exclude rules for this cycle.
    pub(crate) exclusions: Arc<ExclusionRules>,
    /// HTTP clients shared by every request of the cycle, so connections
    /// (and TLS sessions) are reused instead of opened per file.
    pub(crate) http: reqwest::Client,
    pub(crate) http_no_redirect: reqwest::Client,
    /// Folders that received a downloaded file since the last flush. They
    /// are flushed to disk before those downloads are recorded as synced.
    pub(crate) pending_dir_syncs: Arc<std::sync::Mutex<HashSet<PathBuf>>>,
    /// How many downloads may run at once right now (adapts to throttling).
    pub(crate) download_limit: Arc<AdaptiveDownloadLimit>,
}

impl GraphContext {
    /// Notes that `folder` gained a downloaded file whose directory entry
    /// still needs flushing.
    pub(crate) fn note_dir_needs_sync(&self, folder: &Path) {
        if let Ok(mut pending) = self.pending_dir_syncs.lock() {
            pending.insert(folder.to_path_buf());
        }
    }

    /// Flushes every noted folder once. Called before a batch of downloads
    /// is recorded as synced, so a crash cannot lose a file the database
    /// already counts as downloaded.
    pub(crate) fn flush_pending_dir_syncs(&self) -> Result<usize, String> {
        let folders: Vec<PathBuf> = self
            .pending_dir_syncs
            .lock()
            .map_err(|_| "Folder sync lock is poisoned".to_string())?
            .drain()
            .collect();
        for folder in &folders {
            std::fs::File::open(folder)
                .and_then(|handle| handle.sync_all())
                .map_err(|error| {
                    format!("Failed flushing folder '{}': {error}", folder.display())
                })?;
        }
        Ok(folders.len())
    }

    pub(crate) async fn current_access_token(&self) -> String {
        self.access_token.read().await.clone()
    }

    /// Holds every Graph request back until the account's throttle window
    /// (set by `extend_throttle_window`) has passed.
    pub(crate) async fn wait_for_throttle_window(
        &self,
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        loop {
            let resume_at = self
                .throttled_until
                .lock()
                .map_err(|_| "Graph throttle lock is poisoned".to_string())?
                .filter(|resume_at| *resume_at > tokio::time::Instant::now());
            let Some(resume_at) = resume_at else {
                return Ok(());
            };
            sleep_with_cancellation(cancel_flag, resume_at - tokio::time::Instant::now()).await?;
        }
    }

    /// Records a 429 for the whole account: pushes the throttle window out to
    /// `retry_after` from now (never shortens it).
    pub(crate) fn extend_throttle_window(&self, retry_after: Duration) -> Result<(), String> {
        let resume_at = tokio::time::Instant::now() + retry_after;
        let mut throttled_until = self
            .throttled_until
            .lock()
            .map_err(|_| "Graph throttle lock is poisoned".to_string())?;
        if throttled_until.is_none_or(|current| current < resume_at) {
            *throttled_until = Some(resume_at);
            log::warn!(
                "{} [cycle:{}] GRAPH_THROTTLE_WINDOW_EXTENDED wait_ms={}",
                self.account_prefix,
                self.cycle_id,
                retry_after.as_millis()
            );
        }
        drop(throttled_until);
        if let Some(change) = self.download_limit.record_throttle() {
            log::warn!(
                "{} [cycle:{}] DOWNLOAD_LIMIT_LOWERED from={} to={} reason=throttled",
                self.account_prefix,
                self.cycle_id,
                change.from,
                change.to
            );
        }
        Ok(())
    }

    /// Records a finished download, letting the download limit grow.
    pub(crate) fn record_download_success(&self) {
        if let Some(change) = self.download_limit.record_success() {
            log::info!(
                "{} [cycle:{}] DOWNLOAD_LIMIT_RAISED from={} to={} maximum={}",
                self.account_prefix,
                self.cycle_id,
                change.from,
                change.to,
                self.download_limit.maximum()
            );
        }
    }

    pub(crate) async fn refresh_token_if_needed(
        &self,
        stale_token: &str,
    ) -> Result<String, String> {
        let _refresh_guard = self.token_refresh_lock.lock().await;

        let current_token = self.current_access_token().await;
        if !current_token.trim().is_empty() && current_token != stale_token {
            return Ok(current_token);
        }

        let refreshed = match refresh_access_token(&self.profile_id).await {
            Ok(refreshed) => refreshed,
            Err(TokenRefreshError::SignInRequired(detail)) => {
                return Err(record_sign_in_required(
                    &self.profiles_lock,
                    &self.sync_runtime,
                    &self.profile_id,
                    &detail,
                ));
            }
            Err(TokenRefreshError::Failed(detail)) => return Err(detail),
        };
        {
            let mut access_token = self.access_token.write().await;
            *access_token = refreshed.access_token.clone();
        }
        Ok(refreshed.access_token)
    }
}

/// The single path for "this account's saved sign-in no longer works":
/// persists the signed-out state, marks the runtime not auth-ready (which
/// raises the `auth_required` issue) and returns the cycle error.
pub(crate) fn record_sign_in_required(
    profiles_lock: &std::sync::Mutex<()>,
    sync_runtime: &Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: &str,
    detail: &str,
) -> String {
    log::warn!(
        "{} AUTH_SIGN_IN_REQUIRED detail={}",
        log_context::account_prefix(profile_id),
        detail
    );
    if let Err(error) = mark_account_sign_in_required(profiles_lock, profile_id) {
        log::error!(
            "{} AUTH_SIGN_IN_REQUIRED_PERSIST_FAILED error={}",
            log_context::account_prefix(profile_id),
            error
        );
    }
    runtime_set_profile_auth_ready(sync_runtime, profile_id, false);
    format!("Sign-in required: {detail}")
}
