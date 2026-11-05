use super::*;

// Counted progress for a run of deletes, so a long run (thousands of files
// deleted in OneDrive one request at a time) shows "Deleting from OneDrive
// · 120 of 897 files" instead of a bare "Applying changes".

/// How often the count reaches the UI.
const DELETE_PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

pub(crate) struct DeleteProgress {
    sync_runtime: Arc<std::sync::Mutex<SyncRuntimeMap>>,
    profile_id: String,
    cycle_id: String,
    detail: &'static str,
    total: usize,
    done: usize,
    last_report: std::time::Instant,
}

impl DeleteProgress {
    /// Starts counting `total` files (folders are not counted) and shows 0
    /// of `total`. Nothing is shown when there is nothing to delete.
    pub(crate) fn start(graph: &GraphContext, detail: &'static str, total: usize) -> Self {
        let progress = Self {
            sync_runtime: Arc::clone(&graph.sync_runtime),
            profile_id: graph.profile_id.clone(),
            cycle_id: graph.cycle_id.clone(),
            detail,
            total,
            done: 0,
            last_report: std::time::Instant::now(),
        };
        if total > 0 {
            progress.report();
        }
        progress
    }

    /// Counts `files` more files as handled (deleted, or left alone because
    /// they changed since the plan).
    pub(crate) fn advance(&mut self, files: usize) {
        self.done = (self.done + files).min(self.total);
        if self.last_report.elapsed() >= DELETE_PROGRESS_INTERVAL {
            self.last_report = std::time::Instant::now();
            self.report();
        }
    }

    /// Ends the count; the rest of the cycle shows its own activity.
    pub(crate) fn finish(self) {
        if self.total == 0 {
            return;
        }
        runtime_set_current_activity(
            &self.sync_runtime,
            &self.profile_id,
            "applying_local",
            "indeterminate",
            None,
            None,
            None,
            Some("Applying planned changes"),
            Some(&self.cycle_id),
        );
    }

    fn report(&self) {
        runtime_set_current_activity(
            &self.sync_runtime,
            &self.profile_id,
            "applying_local",
            "determinate",
            Some(self.done),
            Some(self.total),
            Some("files"),
            Some(self.detail),
            Some(&self.cycle_id),
        );
    }
}

/// The files (not folders) among `rows`.
pub(crate) fn count_files(rows: &[SyncFileRow]) -> usize {
    rows.iter().filter(|row| !row.is_dir).count()
}
