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
    /// File contents (downloads and uploads), one connection per transfer.
    pub(crate) content_http: reqwest::Client,
    /// Folders that received a downloaded file since the last flush. They
    /// are flushed to disk before those downloads are recorded as synced.
    pub(crate) pending_dir_syncs: Arc<std::sync::Mutex<HashSet<PathBuf>>>,
    /// How many transfers may run at once right now, and how fast they may
    /// start (adapts to throttling). Downloads, uploads and deletes share it:
    /// Microsoft limits the account as a whole.
    pub(crate) download_limit: Arc<AdaptiveDownloadLimit>,
    /// Requests released by the end of a throttle window, counted so the
    /// first goes alone as a probe and the rest are spread out.
    pub(crate) throttle_releases: Arc<AtomicUsize>,
    /// While a probe is out after a throttle window, every other request
    /// holds back until this instant.
    pub(crate) throttle_probe_until: Arc<std::sync::Mutex<Option<tokio::time::Instant>>>,
}

/// Requests released together after a throttle window are spread over this
/// many steps of `THROTTLE_RELEASE_STEP`, so the release itself does not
/// trip the limit again.
const THROTTLE_RELEASE_SPREAD: usize = 10;
const THROTTLE_RELEASE_STEP: Duration = Duration::from_millis(300);
/// When a throttle window ends, one request goes alone and the rest wait
/// this long: if Microsoft refuses the probe, a new window starts and they
/// wait it out instead of all being refused too (each refusal can lengthen
/// the penalty).
const THROTTLE_PROBE_WAIT: Duration = Duration::from_secs(2);

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
    /// (set by `extend_throttle_window`) has passed. Then the first request
    /// held goes alone as a probe; the others wait `THROTTLE_PROBE_WAIT`
    /// and look again (a refused probe has started a new window by then),
    /// and are spread over a few seconds. Returns the moment the request may
    /// start, which the caller passes back if it gets a 429.
    pub(crate) async fn wait_for_throttle_window(
        &self,
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<std::time::Instant, String> {
        let mut release: Option<usize> = None;
        loop {
            let now = tokio::time::Instant::now();
            let resume_at = self
                .throttled_until
                .lock()
                .map_err(|_| "Graph throttle lock is poisoned".to_string())?
                .filter(|resume_at| *resume_at > now);
            if let Some(resume_at) = resume_at {
                sleep_with_cancellation(cancel_flag, resume_at - now).await?;
                // Mark this request as one the window held.
                release = Some(usize::MAX);
                continue;
            }
            if release == Some(usize::MAX) {
                let order = self.throttle_releases.fetch_add(1, Ordering::Relaxed);
                release = Some(order);
                if order == 0 {
                    *self
                        .throttle_probe_until
                        .lock()
                        .map_err(|_| "Graph throttle lock is poisoned".to_string())? =
                        Some(now + THROTTLE_PROBE_WAIT);
                    log::info!(
                        "{} [cycle:{}] GRAPH_THROTTLE_PROBE",
                        self.account_prefix,
                        self.cycle_id
                    );
                    return Ok(self.start_request());
                }
            }
            let probe_until = self
                .throttle_probe_until
                .lock()
                .map_err(|_| "Graph throttle lock is poisoned".to_string())?
                .filter(|until| *until > now);
            let Some(probe_until) = probe_until else {
                return Ok(self.start_request());
            };
            let spread = release
                .and_then(|order| u32::try_from(order % THROTTLE_RELEASE_SPREAD).ok())
                .unwrap_or(0);
            sleep_with_cancellation(
                cancel_flag,
                (probe_until - now) + THROTTLE_RELEASE_STEP * spread,
            )
            .await?;
        }
    }

    /// Counts a request starting now (for the measured request rate) and
    /// returns its start time.
    fn start_request(&self) -> std::time::Instant {
        self.download_limit.record_request_start();
        std::time::Instant::now()
    }

    /// Saves the limits after a change, so the next cycle starts there.
    fn save_learned_rate(&self) {
        let Some(learned) = self.download_limit.learned() else {
            return;
        };
        if let Err(error) = save_learned_download_rate(&self.profile_id, &learned) {
            log::warn!(
                "{} [cycle:{}] DOWNLOAD_RATE_SAVE_FAILED error={}",
                self.account_prefix,
                self.cycle_id,
                error
            );
        }
    }

    /// Reads Microsoft's early warning on a response (`RateLimit-*`
    /// headers, sent once most of a quota is used) and eases the rate
    /// before a 429 comes.
    pub(crate) fn note_rate_limit_headers(&self, headers: &reqwest::header::HeaderMap) {
        let Some(warning) = parse_rate_limit_warning(headers) else {
            return;
        };
        if let Some(change) = self.download_limit.record_pressure() {
            log::warn!(
                "{} [cycle:{}] DOWNLOAD_LIMIT_EASED parallel={} starts_per_second={:.1} reason=rate_limit_warning limit={} remaining={} reset_seconds={}",
                self.account_prefix,
                self.cycle_id,
                change.to,
                change.starts_per_second,
                warning.limit.map_or_else(|| "-".to_string(), |value| value.to_string()),
                warning.remaining.map_or_else(|| "-".to_string(), |value| value.to_string()),
                warning.reset_seconds.map_or_else(|| "-".to_string(), |value| value.to_string())
            );
            self.save_learned_rate();
        }
    }

    /// Records a 429 for the whole account: pushes the throttle window out to
    /// `retry_after` from now (never shortens it) and lowers the download
    /// limits, unless the refused request (started at `request_started_at`)
    /// was sent before the last cut.
    pub(crate) fn extend_throttle_window(
        &self,
        retry_after: Duration,
        request_started_at: std::time::Instant,
    ) -> Result<(), String> {
        let resume_at = tokio::time::Instant::now() + retry_after;
        let mut throttled_until = self
            .throttled_until
            .lock()
            .map_err(|_| "Graph throttle lock is poisoned".to_string())?;
        if throttled_until.is_none_or(|current| current < resume_at) {
            *throttled_until = Some(resume_at);
            // A new window: the next request released goes first, alone.
            self.throttle_releases.store(0, Ordering::Relaxed);
            if let Ok(mut probe_until) = self.throttle_probe_until.lock() {
                *probe_until = None;
            }
            log::warn!(
                "{} [cycle:{}] GRAPH_THROTTLE_WINDOW_EXTENDED wait_ms={}",
                self.account_prefix,
                self.cycle_id,
                retry_after.as_millis()
            );
        }
        drop(throttled_until);
        if let Some(change) = self
            .download_limit
            .record_throttle(request_started_at, retry_after)
        {
            log::warn!(
                "{} [cycle:{}] DOWNLOAD_LIMIT_LOWERED from={} to={} starts_per_second={:.1} reason=throttled",
                self.account_prefix,
                self.cycle_id,
                change.from,
                change.to,
                change.starts_per_second
            );
            self.save_learned_rate();
        }
        Ok(())
    }

    /// Records a finished transfer (download, upload or delete), letting the
    /// account's shared limit grow.
    pub(crate) fn record_transfer_success(&self) {
        if let Some(change) = self.download_limit.record_success() {
            log::info!(
                "{} [cycle:{}] DOWNLOAD_LIMIT_RAISED from={} to={} maximum={} starts_per_second={:.1}",
                self.account_prefix,
                self.cycle_id,
                change.from,
                change.to,
                self.download_limit.maximum(),
                change.starts_per_second
            );
            self.save_learned_rate();
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
