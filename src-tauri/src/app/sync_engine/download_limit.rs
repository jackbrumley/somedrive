use super::*;

/// Parallel downloads a cycle starts with, before it has learned how much
/// Microsoft will take.
const DOWNLOAD_LIMIT_START: usize = 8;
/// The limit never falls below this, however often Microsoft throttles.
const DOWNLOAD_LIMIT_FLOOR: usize = 2;
/// Downloads that must finish without a 429 before the limit rises by one.
const SUCCESSES_PER_RAISE: usize = 20;
/// Throttle answers within this window of a cut count as the same episode,
/// so a burst of parallel 429s halves the limit once, not once per worker.
const THROTTLE_EPISODE: Duration = Duration::from_secs(5);
/// Files at least this big take the large-file lane (see
/// `large_download_slots`).
pub(crate) const LARGE_DOWNLOAD_BYTES: u64 = 8 * 1024 * 1024;

/// How many downloads run at once for one account during a cycle. Starts
/// low, rises by one after every run of successful downloads and halves when
/// Microsoft answers 429, never above the "Parallel downloads" setting.
pub(crate) struct AdaptiveDownloadLimit {
    maximum: usize,
    state: std::sync::Mutex<AdaptiveDownloadLimitState>,
}

struct AdaptiveDownloadLimitState {
    current: usize,
    successes_since_change: usize,
    last_cut_at: Option<std::time::Instant>,
}

/// A change to the limit, for the caller to log.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DownloadLimitChange {
    pub(crate) from: usize,
    pub(crate) to: usize,
}

impl AdaptiveDownloadLimit {
    pub(crate) fn new(maximum: usize) -> Self {
        let maximum = maximum.max(1);
        Self {
            maximum,
            state: std::sync::Mutex::new(AdaptiveDownloadLimitState {
                current: DOWNLOAD_LIMIT_START.min(maximum),
                successes_since_change: 0,
                last_cut_at: None,
            }),
        }
    }

    pub(crate) fn maximum(&self) -> usize {
        self.maximum
    }

    pub(crate) fn current(&self) -> usize {
        self.state
            .lock()
            .map(|state| state.current)
            .unwrap_or(DOWNLOAD_LIMIT_FLOOR.min(self.maximum))
    }

    /// Slots kept for files of `LARGE_DOWNLOAD_BYTES` or more: a quarter of
    /// the limit (at least one), so a few big files cannot hold every slot
    /// while thousands of small ones wait.
    pub(crate) fn large_download_slots(&self) -> usize {
        (self.current() / 4).max(1)
    }

    /// Records a finished download; returns the change when it raises the
    /// limit.
    pub(crate) fn record_success(&self) -> Option<DownloadLimitChange> {
        let mut state = self.state.lock().ok()?;
        if state.current >= self.maximum {
            return None;
        }
        state.successes_since_change += 1;
        if state.successes_since_change < SUCCESSES_PER_RAISE {
            return None;
        }
        let from = state.current;
        state.current += 1;
        state.successes_since_change = 0;
        Some(DownloadLimitChange {
            from,
            to: state.current,
        })
    }

    /// Records a 429; halves the limit once per throttle episode and returns
    /// the change when it lowered it.
    pub(crate) fn record_throttle(&self) -> Option<DownloadLimitChange> {
        let mut state = self.state.lock().ok()?;
        let now = std::time::Instant::now();
        state.successes_since_change = 0;
        if state
            .last_cut_at
            .is_some_and(|cut_at| now.duration_since(cut_at) < THROTTLE_EPISODE)
        {
            return None;
        }
        state.last_cut_at = Some(now);
        let floor = DOWNLOAD_LIMIT_FLOOR.min(self.maximum);
        let halved = (state.current / 2).max(floor);
        if halved == state.current {
            return None;
        }
        let from = state.current;
        state.current = halved;
        Some(DownloadLimitChange { from, to: halved })
    }
}

#[cfg(test)]
mod adaptive_limit_tests {
    use super::*;

    #[test]
    fn starts_at_eight_or_the_setting_when_lower() {
        assert_eq!(AdaptiveDownloadLimit::new(12).current(), 8);
        assert_eq!(AdaptiveDownloadLimit::new(4).current(), 4);
    }

    #[test]
    fn rises_by_one_per_run_of_successes_up_to_the_setting() {
        let limit = AdaptiveDownloadLimit::new(9);
        for _ in 0..SUCCESSES_PER_RAISE - 1 {
            assert_eq!(limit.record_success(), None);
        }
        assert_eq!(
            limit.record_success(),
            Some(DownloadLimitChange { from: 8, to: 9 })
        );
        for _ in 0..SUCCESSES_PER_RAISE * 2 {
            assert_eq!(limit.record_success(), None, "never above the setting");
        }
        assert_eq!(limit.current(), 9);
    }

    #[test]
    fn a_burst_of_429s_halves_once_and_never_below_the_floor() {
        let limit = AdaptiveDownloadLimit::new(32);
        assert_eq!(
            limit.record_throttle(),
            Some(DownloadLimitChange { from: 8, to: 4 })
        );
        assert_eq!(limit.record_throttle(), None, "same episode");
        assert_eq!(limit.current(), 4);

        let limit = AdaptiveDownloadLimit::new(32);
        for _ in 0..5 {
            limit.state.lock().expect("state").last_cut_at = None;
            limit.record_throttle();
        }
        assert_eq!(limit.current(), DOWNLOAD_LIMIT_FLOOR);
    }

    #[test]
    fn a_429_restarts_the_run_of_successes() {
        let limit = AdaptiveDownloadLimit::new(32);
        for _ in 0..SUCCESSES_PER_RAISE - 1 {
            limit.record_success();
        }
        limit.record_throttle();
        assert_eq!(limit.record_success(), None);
    }

    #[test]
    fn large_files_get_a_quarter_of_the_slots() {
        assert_eq!(AdaptiveDownloadLimit::new(32).large_download_slots(), 2);
        assert_eq!(AdaptiveDownloadLimit::new(2).large_download_slots(), 1);
    }
}
