use super::*;
use notify::event::{AccessKind, AccessMode};
use notify::{EventKind, RecursiveMode, Watcher};

// Watching the sync folder (inotify) instead of rescanning it every cycle
// (docs/sync-engine.md, "Finding local changes"). The watcher only answers
// one question: does this cycle need to scan the device? The scan itself
// is unchanged, so the watcher can never make the engine wrong, only slower
// to notice: every doubt (no watcher, a failed one, a dropped event, new
// exclude rules, another folder) means a full scan, and a full scan runs
// every `FULL_SCAN_INTERVAL` whatever the watcher says.
//
// The engine records its own downloads, uploads and deletes in the
// database as it makes them, so a cycle that skips the scan still plans
// from what is on disk.

/// After a change, the folder must be quiet this long before a cycle
/// starts for it, so a file being copied in is not picked up half-written.
pub(crate) const LOCAL_CHANGE_SETTLE: Duration = Duration::from_secs(2);
/// A full scan runs at least this often, to catch anything the watcher
/// missed.
pub(crate) const FULL_SCAN_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// What the watcher thread tells the sync worker.
#[derive(Default)]
struct WatchSignals {
    /// Something changed since the last scan started.
    changed: AtomicBool,
    /// When the last change was seen (for the settle time).
    last_change: std::sync::Mutex<Option<std::time::Instant>>,
    /// The watcher reported an error: it can no longer be trusted.
    failed: AtomicBool,
}

impl WatchSignals {
    fn note_change(&self) {
        self.changed.store(true, Ordering::SeqCst);
        if let Ok(mut last_change) = self.last_change.lock() {
            *last_change = Some(std::time::Instant::now());
        }
    }
}

/// Why a cycle scans the device (logged with the scan).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalScanReason {
    /// The first cycle since the worker started (or the folder changed).
    FirstScan,
    /// The folder could not be watched, or the watcher failed.
    NotWatched,
    /// The exclude rules changed, which changes what a scan sees.
    ExcludeRulesChanged,
    /// The safety-net full scan is due.
    Periodic,
    /// The watcher saw changes.
    ChangesSeen,
}

impl LocalScanReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::FirstScan => "first_scan",
            Self::NotWatched => "not_watched",
            Self::ExcludeRulesChanged => "exclude_rules_changed",
            Self::Periodic => "periodic",
            Self::ChangesSeen => "changes_seen",
        }
    }
}

/// One account's sync folder watcher, owned by its sync worker.
pub(crate) struct LocalChangeWatch {
    profile_id: String,
    signals: Arc<WatchSignals>,
    watcher: Option<notify::RecommendedWatcher>,
    watched_root: Option<PathBuf>,
    scanned_patterns: Option<Vec<String>>,
    last_full_scan: Option<std::time::Instant>,
    /// The watching state last saved for the account page.
    reported_watching: Option<bool>,
}

impl LocalChangeWatch {
    pub(crate) fn new(profile_id: &str) -> Self {
        Self {
            profile_id: profile_id.to_string(),
            signals: Arc::new(WatchSignals::default()),
            watcher: None,
            watched_root: None,
            scanned_patterns: None,
            last_full_scan: None,
            reported_watching: None,
        }
    }

    /// The watching state, when it differs from the one last reported.
    pub(crate) fn unreported_watching_state(&mut self) -> Option<bool> {
        let watching = self.is_watching();
        (self.reported_watching != Some(watching)).then(|| {
            self.reported_watching = Some(watching);
            watching
        })
    }

    /// Whether changes on this device are being watched right now.
    pub(crate) fn is_watching(&self) -> bool {
        self.watcher.is_some() && !self.signals.failed.load(Ordering::SeqCst)
    }

    /// True when the watcher saw changes and the folder has since been
    /// quiet for `LOCAL_CHANGE_SETTLE`: the worker starts a cycle now
    /// instead of waiting for the next timed one.
    pub(crate) fn changes_settled(&self) -> bool {
        if !self.is_watching() || !self.signals.changed.load(Ordering::SeqCst) {
            return false;
        }
        self.signals
            .last_change
            .lock()
            .ok()
            .and_then(|last_change| *last_change)
            .is_some_and(|last_change| last_change.elapsed() >= LOCAL_CHANGE_SETTLE)
    }

    /// Called at the start of every cycle: watches `sync_root` (again, if
    /// the folder changed) and says whether this cycle must scan the device.
    /// A scan clears the "changed" mark before it starts, so a change made
    /// while it runs is seen by the next cycle.
    pub(crate) fn scan_decision(
        &mut self,
        sync_root: &Path,
        exclusion_patterns: &[String],
    ) -> Option<LocalScanReason> {
        let first = self.watched_root.as_deref() != Some(sync_root);
        let full_scan_due = self
            .last_full_scan
            .is_none_or(|scanned_at| scanned_at.elapsed() >= FULL_SCAN_INTERVAL);
        // A watcher that could not start or failed is tried again with the
        // safety-net scan, not every cycle (re-adding thousands of watches
        // every 15 s against a full system limit would be wasted work).
        if first || (full_scan_due && !self.is_watching()) {
            self.start(sync_root);
        }
        let reason = if first {
            Some(LocalScanReason::FirstScan)
        } else if !self.is_watching() {
            Some(LocalScanReason::NotWatched)
        } else if self.scanned_patterns.as_deref() != Some(exclusion_patterns) {
            Some(LocalScanReason::ExcludeRulesChanged)
        } else if full_scan_due {
            Some(LocalScanReason::Periodic)
        } else if self.signals.changed.load(Ordering::SeqCst) {
            Some(LocalScanReason::ChangesSeen)
        } else {
            None
        };
        if reason.is_some() {
            self.signals.changed.store(false, Ordering::SeqCst);
            self.last_full_scan = Some(std::time::Instant::now());
            self.scanned_patterns = Some(exclusion_patterns.to_vec());
        }
        reason
    }

    /// Starts watching `sync_root` and everything under it, replacing any
    /// earlier watcher. Where that is not possible (no such folder, the
    /// system's watch limit reached), every cycle scans, as before.
    fn start(&mut self, sync_root: &Path) {
        self.watcher = None;
        self.watched_root = Some(sync_root.to_path_buf());
        self.signals = Arc::new(WatchSignals::default());
        let signals = Arc::clone(&self.signals);
        let account_prefix = log_context::account_prefix(&self.profile_id);
        let handler_prefix = account_prefix.clone();
        let started = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
            match result {
                Ok(event) => {
                    if event.need_rescan() {
                        log::warn!("{handler_prefix} LOCAL_WATCH_EVENTS_DROPPED action=full_scan");
                        signals.note_change();
                    } else if is_relevant_change(&event) {
                        signals.note_change();
                    }
                }
                Err(error) => {
                    if !signals.failed.swap(true, Ordering::SeqCst) {
                        log::warn!(
                            "{handler_prefix} LOCAL_WATCH_FAILED error={error} action=scan_every_cycle"
                        );
                    }
                }
            }
        })
        .and_then(|mut watcher| {
            watcher
                .watch(sync_root, RecursiveMode::Recursive)
                .map(|()| watcher)
        });
        match started {
            Ok(watcher) => {
                log::info!(
                    "{} LOCAL_WATCH_STARTED sync_root={}",
                    account_prefix,
                    sync_root.display()
                );
                self.watcher = Some(watcher);
            }
            Err(error) => log::warn!(
                "{} LOCAL_WATCH_UNAVAILABLE sync_root={} error={} action=scan_every_cycle",
                account_prefix,
                sync_root.display(),
                error
            ),
        }
    }
}

/// Saves whether the account's folder is watched (the status shows it).
pub(crate) fn persist_local_watch_state(profile_id: &str, watching: bool) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "INSERT INTO sync_local_watch (profile_id, watching, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(profile_id) DO UPDATE SET
                 watching = excluded.watching,
                 updated_at = excluded.updated_at",
            params![profile_id, bool_to_sql(watching), current_unix_seconds()],
        )
        .map_err(|error| format!("Failed saving the folder watch state: {error}"))?;
    Ok(())
}

/// Whether the account's folder is watched; `None` before the first cycle.
pub(crate) fn read_local_watch_state(connection: &Connection) -> Result<Option<bool>, String> {
    connection
        .query_row("SELECT watching FROM sync_local_watch LIMIT 1", [], |row| {
            row.get::<_, i64>(0)
        })
        .optional()
        .map(|watching| watching.map(|value| value != 0))
        .map_err(|error| format!("Failed reading the folder watch state: {error}"))
}

/// A change worth a scan: not a read, and not one of SomeDrive's own
/// temporary download files (finished downloads are recorded by the
/// engine itself).
fn is_relevant_change(event: &notify::Event) -> bool {
    let read_only = matches!(
        event.kind,
        EventKind::Access(kind) if kind != AccessKind::Close(AccessMode::Write)
    );
    if read_only {
        return false;
    }
    event.paths.is_empty() || event.paths.iter().any(|path| !is_own_temp_path(path))
}

fn is_own_temp_path(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|name| is_download_temp_name(name) || is_resumable_partial_name(name))
    })
}

#[cfg(test)]
mod local_watch_tests {
    use super::*;

    fn temp_folder(label: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "somedrive-watch-{label}-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&folder).expect("folder");
        folder
    }

    fn wait_for(mut condition: impl FnMut() -> bool) -> bool {
        for _ in 0..100 {
            if condition() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn the_first_cycle_scans_then_only_changes_do() {
        let folder = temp_folder("decide");
        let mut watch = LocalChangeWatch::new("watch-test");
        assert_eq!(
            watch.scan_decision(&folder, &[]),
            Some(LocalScanReason::FirstScan)
        );
        assert!(watch.is_watching());
        assert_eq!(watch.scan_decision(&folder, &[]), None, "nothing changed");

        std::fs::write(folder.join("new.txt"), b"hello").expect("write");
        assert!(wait_for(|| watch.signals.changed.load(Ordering::SeqCst)));
        assert_eq!(
            watch.scan_decision(&folder, &[]),
            Some(LocalScanReason::ChangesSeen)
        );
        assert_eq!(
            watch.scan_decision(&folder, &[]),
            None,
            "the scan cleared it"
        );

        // A change in a folder created after the watch started is seen too.
        std::fs::create_dir(folder.join("later")).expect("dir");
        assert!(wait_for(|| watch.signals.changed.load(Ordering::SeqCst)));
        watch.scan_decision(&folder, &[]);
        std::thread::sleep(Duration::from_millis(200));
        watch.signals.changed.store(false, Ordering::SeqCst);
        std::fs::write(folder.join("later/deep.txt"), b"x").expect("write");
        assert!(wait_for(|| watch.signals.changed.load(Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn new_exclude_rules_or_another_folder_mean_a_full_scan() {
        let folder = temp_folder("rules");
        let other = temp_folder("other");
        let mut watch = LocalChangeWatch::new("watch-test-rules");
        watch.scan_decision(&folder, &[]);
        assert_eq!(
            watch.scan_decision(&folder, &["*.tmp".to_string()]),
            Some(LocalScanReason::ExcludeRulesChanged)
        );
        assert_eq!(
            watch.scan_decision(&other, &["*.tmp".to_string()]),
            Some(LocalScanReason::FirstScan)
        );
        let _ = std::fs::remove_dir_all(&folder);
        let _ = std::fs::remove_dir_all(&other);
    }

    #[test]
    fn a_folder_that_cannot_be_watched_scans_every_cycle() {
        let missing = std::env::temp_dir().join("somedrive-watch-missing-folder-for-test");
        let mut watch = LocalChangeWatch::new("watch-test-missing");
        assert_eq!(
            watch.scan_decision(&missing, &[]),
            Some(LocalScanReason::FirstScan)
        );
        assert!(!watch.is_watching());
        assert_eq!(
            watch.scan_decision(&missing, &[]),
            Some(LocalScanReason::NotWatched)
        );
    }

    #[test]
    fn own_temporary_downloads_and_reads_are_not_changes() {
        assert!(is_own_temp_path(Path::new(
            "/s/Docs/.somedrive-part-abc.tmp"
        )));
        assert!(is_own_temp_path(Path::new(
            "/s/Docs/.somedrive-partial-key.part/data.part"
        )));
        assert!(!is_own_temp_path(Path::new("/s/Docs/report.docx")));
        let read = notify::Event::new(EventKind::Access(AccessKind::Open(
            notify::event::AccessMode::Read,
        )))
        .add_path(PathBuf::from("/s/Docs/report.docx"));
        assert!(!is_relevant_change(&read));
        let written = notify::Event::new(EventKind::Access(AccessKind::Close(AccessMode::Write)))
            .add_path(PathBuf::from("/s/Docs/report.docx"));
        assert!(is_relevant_change(&written));
    }
}
