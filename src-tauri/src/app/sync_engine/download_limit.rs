use super::*;
use std::time::Instant;

/// Parallel downloads a cycle starts with, before it has learned how much
/// Microsoft will take.
const DOWNLOAD_LIMIT_START: usize = 8;
/// The limit never falls below this, however often Microsoft throttles.
const DOWNLOAD_LIMIT_FLOOR: usize = 4;
/// The limits rise by at most one small step per this interval, and only
/// while downloads are succeeding: a steady creep upwards, never a jump, so
/// the rate settles just under Microsoft's limit instead of swinging between
/// fast and throttled.
const RAISE_INTERVAL: Duration = Duration::from_secs(10);
/// Each step raises the start rate by this fraction (and parallel downloads
/// by one).
const START_RATE_STEP: f64 = 0.10;
/// New downloads that may start per second at first. Microsoft limits files
/// per minute as well as bandwidth: thousands of tiny files started as fast
/// as slots free up use that quota in seconds.
const START_RATE_INITIAL: f64 = 8.0;
const START_RATE_FLOOR: f64 = 2.0;
const START_RATE_CEILING: f64 = 40.0;
/// Files at least this big take the large-file lane (see
/// `large_download_slots`).
pub(crate) const LARGE_DOWNLOAD_BYTES: u64 = 8 * 1024 * 1024;

/// How fast one account downloads during a cycle: how many downloads run at
/// once, and how many may start per second. Both start low, creep up one
/// small step per `RAISE_INTERVAL` while downloads succeed, and drop to three
/// quarters when Microsoft answers 429. A 429 only counts when its request
/// started after the last cut: answers to requests sent at the old rate say
/// nothing about the new one.
pub(crate) struct AdaptiveDownloadLimit {
    maximum: usize,
    state: std::sync::Mutex<AdaptiveDownloadLimitState>,
}

struct AdaptiveDownloadLimitState {
    current: usize,
    starts_per_second: f64,
    start_permits: f64,
    permits_refilled_at: Instant,
    successes_since_change: usize,
    last_change_at: Instant,
    last_cut_at: Option<Instant>,
}

/// A change to the limits, for the caller to log.
#[derive(Debug, PartialEq)]
pub(crate) struct DownloadLimitChange {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) starts_per_second: f64,
}

impl AdaptiveDownloadLimit {
    pub(crate) fn new(maximum: usize) -> Self {
        let maximum = maximum.max(1);
        Self {
            maximum,
            state: std::sync::Mutex::new(AdaptiveDownloadLimitState {
                current: DOWNLOAD_LIMIT_START.min(maximum),
                starts_per_second: START_RATE_INITIAL,
                start_permits: START_RATE_INITIAL,
                permits_refilled_at: Instant::now(),
                successes_since_change: 0,
                last_change_at: Instant::now(),
                last_cut_at: None,
            }),
        }
    }

    pub(crate) fn maximum(&self) -> usize {
        self.maximum
    }

    fn floor(&self) -> usize {
        DOWNLOAD_LIMIT_FLOOR.min(self.maximum)
    }

    pub(crate) fn current(&self) -> usize {
        self.state
            .lock()
            .map(|state| state.current)
            .unwrap_or_else(|_| self.floor())
    }

    /// Slots kept for files of `LARGE_DOWNLOAD_BYTES` or more: a quarter of
    /// the limit (at least one), so a few big files cannot hold every slot
    /// while thousands of small ones wait.
    pub(crate) fn large_download_slots(&self) -> usize {
        (self.current() / 4).max(1)
    }

    /// How many of `wanted` new downloads may start now under the start
    /// rate (a token bucket holding at most one second of starts, so a
    /// queue released after a throttle wait restarts gradually).
    pub(crate) fn take_start_permits(&self, wanted: usize) -> usize {
        self.take_start_permits_at(wanted, Instant::now())
    }

    fn take_start_permits_at(&self, wanted: usize, now: Instant) -> usize {
        let Ok(mut state) = self.state.lock() else {
            return 0;
        };
        let elapsed = now
            .saturating_duration_since(state.permits_refilled_at)
            .as_secs_f64();
        state.permits_refilled_at = now;
        state.start_permits =
            (state.start_permits + elapsed * state.starts_per_second).min(state.starts_per_second);
        let granted = (state.start_permits.floor() as usize).min(wanted);
        state.start_permits -= granted as f64;
        granted
    }

    /// Records a finished download; returns the change when it raises the
    /// limits (at most once per `RAISE_INTERVAL`).
    pub(crate) fn record_success(&self) -> Option<DownloadLimitChange> {
        self.record_success_at(Instant::now())
    }

    fn record_success_at(&self, now: Instant) -> Option<DownloadLimitChange> {
        let mut state = self.state.lock().ok()?;
        state.successes_since_change += 1;
        let at_ceiling =
            state.current >= self.maximum && state.starts_per_second >= START_RATE_CEILING;
        let step_due = now.saturating_duration_since(state.last_change_at) >= RAISE_INTERVAL;
        if at_ceiling || !step_due {
            return None;
        }
        let from = state.current;
        state.current = (state.current + 1).min(self.maximum);
        state.starts_per_second =
            (state.starts_per_second * (1.0 + START_RATE_STEP)).min(START_RATE_CEILING);
        state.successes_since_change = 0;
        state.last_change_at = now;
        Some(DownloadLimitChange {
            from,
            to: state.current,
            starts_per_second: state.starts_per_second,
        })
    }

    /// Hands back start permits that were granted but not used (the queue
    /// had fewer jobs ready), so they are not lost.
    pub(crate) fn return_start_permits(&self, unused: usize) {
        if unused == 0 {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            state.start_permits =
                (state.start_permits + unused as f64).min(state.starts_per_second);
        }
    }

    /// Records a 429 for a request that started at `request_started_at`.
    /// Lowers both limits to three quarters, unless the request was sent
    /// before the last cut. Returns the change when it lowered them.
    pub(crate) fn record_throttle(
        &self,
        request_started_at: Instant,
    ) -> Option<DownloadLimitChange> {
        self.record_throttle_at(request_started_at, Instant::now())
    }

    fn record_throttle_at(
        &self,
        request_started_at: Instant,
        now: Instant,
    ) -> Option<DownloadLimitChange> {
        let mut state = self.state.lock().ok()?;
        state.successes_since_change = 0;
        if state
            .last_cut_at
            .is_some_and(|cut_at| request_started_at <= cut_at)
        {
            return None;
        }
        state.last_cut_at = Some(now);
        state.last_change_at = now;
        let from = state.current;
        state.current = (state.current * 3 / 4).max(self.floor());
        state.starts_per_second = (state.starts_per_second * 0.75).max(START_RATE_FLOOR);
        state.start_permits = state.start_permits.min(state.starts_per_second);
        Some(DownloadLimitChange {
            from,
            to: state.current,
            starts_per_second: state.starts_per_second,
        })
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
    fn rises_one_small_step_per_interval_and_never_above_the_setting() {
        let limit = AdaptiveDownloadLimit::new(9);
        let start = Instant::now();
        for _ in 0..100 {
            assert_eq!(
                limit.record_success_at(start),
                None,
                "a burst of successes is not a step"
            );
        }
        let change = limit
            .record_success_at(start + RAISE_INTERVAL)
            .expect("one step after the interval");
        assert_eq!((change.from, change.to), (8, 9));
        assert!((change.starts_per_second - START_RATE_INITIAL * 1.1).abs() < 1e-9);
        assert_eq!(
            limit.record_success_at(start + RAISE_INTERVAL + Duration::from_secs(1)),
            None,
            "the next step waits a full interval"
        );
        for step in 2..10 {
            limit.record_success_at(start + RAISE_INTERVAL * step);
        }
        assert_eq!(limit.current(), 9, "never above the setting");
    }

    #[test]
    fn a_cut_restarts_the_interval() {
        let limit = AdaptiveDownloadLimit::new(32);
        let sent = Instant::now();
        let cut_at = sent + RAISE_INTERVAL;
        limit.record_throttle_at(sent, cut_at).expect("cut");
        assert_eq!(
            limit.record_success_at(cut_at + Duration::from_secs(1)),
            None
        );
        assert!(limit.record_success_at(cut_at + RAISE_INTERVAL).is_some());
    }

    #[test]
    fn a_429_from_a_request_sent_before_the_cut_is_ignored() {
        let limit = AdaptiveDownloadLimit::new(32);
        let sent_early = Instant::now();
        let first_cut = sent_early + Duration::from_millis(10);
        let change = limit
            .record_throttle_at(sent_early, first_cut)
            .expect("cut");
        assert_eq!((change.from, change.to), (8, 6), "three quarters");

        let late_answer = first_cut + Duration::from_secs(6);
        assert_eq!(
            limit.record_throttle_at(sent_early, late_answer),
            None,
            "sent at the old rate"
        );
        assert_eq!(limit.current(), 6);

        let sent_after = first_cut + Duration::from_secs(1);
        assert!(limit.record_throttle_at(sent_after, late_answer).is_some());
    }

    #[test]
    fn never_below_the_floor() {
        let limit = AdaptiveDownloadLimit::new(32);
        let mut now = Instant::now();
        for _ in 0..10 {
            let sent = now + Duration::from_millis(1);
            now = sent + Duration::from_millis(1);
            limit.record_throttle_at(sent, now);
        }
        assert_eq!(limit.current(), DOWNLOAD_LIMIT_FLOOR);
    }

    #[test]
    fn starts_are_paced_to_the_start_rate() {
        let limit = AdaptiveDownloadLimit::new(32);
        let now = Instant::now();
        assert_eq!(
            limit.take_start_permits_at(100, now),
            8,
            "one second's worth"
        );
        assert_eq!(limit.take_start_permits_at(100, now), 0);
        let half_second = now + Duration::from_millis(500);
        assert_eq!(limit.take_start_permits_at(100, half_second), 4);
        limit.return_start_permits(3);
        assert_eq!(
            limit.take_start_permits_at(100, half_second),
            3,
            "unused permits come back"
        );
        let much_later = half_second + Duration::from_secs(60);
        assert_eq!(
            limit.take_start_permits_at(100, much_later),
            8,
            "a long wait still releases at most one second's worth"
        );
    }

    #[test]
    fn large_files_get_a_quarter_of_the_slots() {
        assert_eq!(AdaptiveDownloadLimit::new(32).large_download_slots(), 2);
        assert_eq!(AdaptiveDownloadLimit::new(2).large_download_slots(), 1);
    }
}
