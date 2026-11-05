use super::*;

// Watches the change counters (docs/ui-data-flow.md) on the status emitter
// thread. Whatever changed what the window shows, and whether or not it
// asked for a refresh, the window hears about it within one check:
// - an account's database counter moving sends that account's status
//   (which carries the counters, so the window reloads that area);
// - the app data revision moving (accounts, settings, sign-in) sends
//   `app-data-changed`.

/// How often the account databases' counters are read. The app data
/// revision is an in-memory number, checked on every emitter tick.
pub(crate) const CHANGE_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

pub(crate) const APP_DATA_EVENT_NAME: &str = "app-data-changed";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppDataChangedEvent {
    pub revision: u64,
}

#[derive(Default)]
pub(crate) struct ChangeWatcher {
    account_revisions: HashMap<String, DataRevisions>,
    /// The last read error per account, so a broken database is logged once
    /// rather than every check.
    account_errors: HashMap<String, String>,
    app_data_revision: u64,
}

impl ChangeWatcher {
    /// Reads every account's counters and asks for the status of each
    /// account whose counters moved.
    pub(crate) fn check_accounts(&mut self, sync_runtime: &Mutex<SyncRuntimeMap>) {
        let profile_ids: Vec<String> = match sync_runtime.lock() {
            Ok(runtime_map) => runtime_map.keys().cloned().collect(),
            Err(_) => return,
        };
        for profile_id in self.accounts_with_moved_counters(&profile_ids) {
            request_status_event(&profile_id, StatusEventEmitMode::Immediate);
        }
    }

    /// Sends `app-data-changed` when the app data revision moved.
    pub(crate) fn check_app_data(&mut self) {
        let app_data_revision = crate::app::json_store::app_data_revision();
        if app_data_revision != self.app_data_revision {
            self.app_data_revision = app_data_revision;
            emit_app_data_changed(app_data_revision);
        }
    }

    /// The accounts whose counters moved since the last check (all of them
    /// on the first check).
    fn accounts_with_moved_counters(&mut self, profile_ids: &[String]) -> Vec<String> {
        self.account_revisions
            .retain(|profile_id, _| profile_ids.contains(profile_id));
        self.account_errors
            .retain(|profile_id, _| profile_ids.contains(profile_id));
        let mut moved = Vec::new();
        for profile_id in profile_ids {
            match crate::app::sync_engine::read_account_data_revisions(profile_id) {
                Ok(revisions) => {
                    self.account_errors.remove(profile_id);
                    if self.account_revisions.insert(profile_id.clone(), revisions)
                        != Some(revisions)
                    {
                        moved.push(profile_id.clone());
                    }
                }
                Err(error) => {
                    if self.account_errors.get(profile_id) != Some(&error) {
                        log::warn!(
                            "{} CHANGE_COUNTERS_READ_FAILED error={}",
                            crate::app::log_context::account_prefix(profile_id),
                            error
                        );
                        self.account_errors.insert(profile_id.clone(), error);
                    }
                }
            }
        }
        moved
    }
}

fn emit_app_data_changed(revision: u64) {
    let app_handle = SYNC_STATUS_APP_HANDLE
        .lock()
        .ok()
        .and_then(|handle| (*handle).clone());
    if let Some(app_handle) = app_handle {
        if let Err(error) = app_handle.emit(APP_DATA_EVENT_NAME, AppDataChangedEvent { revision }) {
            log::warn!("APP_DATA_EVENT_EMIT_FAILED error={}", error);
        }
    }
}

#[cfg(test)]
mod change_watcher_tests {
    use super::*;

    #[test]
    fn a_counter_moved_by_any_write_is_noticed_once() {
        let profile_id = format!(
            "change-watcher-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let profile_ids = vec![profile_id.clone()];
        let mut watcher = ChangeWatcher::default();
        assert_eq!(
            watcher.accounts_with_moved_counters(&profile_ids),
            profile_ids,
            "first look"
        );
        assert!(
            watcher
                .accounts_with_moved_counters(&profile_ids)
                .is_empty(),
            "nothing changed"
        );

        // A write that asks for no refresh at all.
        crate::app::sync_engine::hide_cleanup_path(&profile_id, "Photos/a.png").expect("hide");
        assert_eq!(
            watcher.accounts_with_moved_counters(&profile_ids),
            profile_ids
        );
        assert!(watcher
            .accounts_with_moved_counters(&profile_ids)
            .is_empty());
    }
}
