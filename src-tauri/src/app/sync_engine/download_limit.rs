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
/// A cut covers Microsoft's whole penalty: the "wait N seconds" it asked
/// for, the staggered release of the requests it held, and this much grace.
/// Microsoft keeps refusing for a moment after the wait, so without it each
/// request released after the wait would count as a new cut.
const CUT_GRACE: Duration = Duration::from_secs(10);
/// After a 429 the limits climb back (one step a minute) only to this
/// fraction of where Microsoft refused, so the rate settles under the level
/// it tolerates instead of hitting it again (each 429 costs a full stop of
/// up to 30 s).
const SETTLE_FRACTION: f64 = 0.8;
/// Above that, the limits only probe upwards slowly: one small step per
/// interval, in case Microsoft now allows more.
const PROBE_INTERVAL: Duration = Duration::from_secs(60);
const PROBE_STEP: f64 = 0.02;
/// Files at least this big take the large-file lane (see
/// `large_download_slots`).
pub(crate) const LARGE_DOWNLOAD_BYTES: u64 = 8 * 1024 * 1024;

/// How fast one account downloads during a cycle: how many downloads run at
/// once, and how many may start per second. Both start low, creep up one
/// small step per `RAISE_INTERVAL` while downloads succeed, and drop to three
/// quarters when Microsoft answers 429. One cut covers the whole penalty
/// (`CUT_GRACE`): a 429 only counts when its request started after that, so
/// one throttle episode is one cut, never a cascade. The level of the last
/// 429 is remembered: the limits climb back one step a minute to
/// `SETTLE_FRACTION` of it and then only probe above it in small steps, so
/// the rate settles instead of swinging.
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
    /// 429s for requests started before this belong to the last cut.
    cut_covers_until: Option<Instant>,
    /// Parallel downloads and starts per second when Microsoft last refused.
    throttled_at: Option<(usize, f64)>,
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
                cut_covers_until: None,
                throttled_at: None,
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
    /// limits: one step per `RAISE_INTERVAL` up to `SETTLE_FRACTION` of the
    /// last 429's level, then one small step per `PROBE_INTERVAL`.
    pub(crate) fn record_success(&self) -> Option<DownloadLimitChange> {
        self.record_success_at(Instant::now())
    }

    fn record_success_at(&self, now: Instant) -> Option<DownloadLimitChange> {
        let mut state = self.state.lock().ok()?;
        state.successes_since_change += 1;
        let at_ceiling =
            state.current >= self.maximum && state.starts_per_second >= START_RATE_CEILING;
        let settle_point = state.throttled_at.map(|(parallel, rate)| {
            (
                ((parallel as f64) * SETTLE_FRACTION).floor() as usize,
                rate * SETTLE_FRACTION,
            )
        });
        let probing = settle_point.is_some_and(|(parallel, rate)| {
            state.current >= parallel || state.starts_per_second >= rate
        });
        // Before the first 429: a step every 10 s. After any 429: a step a
        // minute (back to the settle point in normal-sized steps, then small
        // probes above it), since climbing back fast just meets the limit
        // again.
        let interval = if settle_point.is_some() {
            PROBE_INTERVAL
        } else {
            RAISE_INTERVAL
        };
        let rate_step = if probing { PROBE_STEP } else { START_RATE_STEP };
        let step_due = now.saturating_duration_since(state.last_change_at) >= interval;
        if at_ceiling || !step_due {
            return None;
        }
        let mut next_parallel = (state.current + 1).min(self.maximum);
        let mut next_rate = (state.starts_per_second * (1.0 + rate_step)).min(START_RATE_CEILING);
        if let (false, Some((parallel, rate))) = (probing, settle_point) {
            // Climbing back: stop at the settle point, never past it.
            next_parallel = next_parallel.min(parallel.max(state.current));
            next_rate = next_rate.min(rate.max(state.starts_per_second));
        }
        let from = state.current;
        state.current = next_parallel;
        state.starts_per_second = next_rate;
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

    /// Records a 429 for a request that started at `request_started_at`,
    /// answered with "wait `retry_after`". Lowers both limits to three
    /// quarters, unless the request started while the last cut's penalty
    /// was still running. Returns the change when it lowered them.
    pub(crate) fn record_throttle(
        &self,
        request_started_at: Instant,
        retry_after: Duration,
    ) -> Option<DownloadLimitChange> {
        self.record_throttle_at(request_started_at, retry_after, Instant::now())
    }

    fn record_throttle_at(
        &self,
        request_started_at: Instant,
        retry_after: Duration,
        now: Instant,
    ) -> Option<DownloadLimitChange> {
        let mut state = self.state.lock().ok()?;
        state.successes_since_change = 0;
        if state
            .cut_covers_until
            .is_some_and(|covered| request_started_at <= covered)
        {
            return None;
        }
        state.cut_covers_until = Some(now + retry_after + CUT_GRACE);
        state.last_change_at = now + retry_after;
        state.throttled_at = Some((state.current, state.starts_per_second));
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
        limit
            .record_throttle_at(sent, Duration::ZERO, cut_at)
            .expect("cut");
        assert_eq!(
            limit.record_success_at(cut_at + Duration::from_secs(1)),
            None
        );
        assert!(limit.record_success_at(cut_at + PROBE_INTERVAL).is_some());
    }

    #[test]
    fn a_429_from_a_request_sent_before_the_cut_is_ignored() {
        let limit = AdaptiveDownloadLimit::new(32);
        let sent_early = Instant::now();
        let first_cut = sent_early + Duration::from_millis(10);
        let change = limit
            .record_throttle_at(sent_early, Duration::ZERO, first_cut)
            .expect("cut");
        assert_eq!((change.from, change.to), (8, 6), "three quarters");

        let late_answer = first_cut + Duration::from_secs(6);
        let after_the_penalty = first_cut + CUT_GRACE + Duration::from_secs(1);
        assert_eq!(
            limit.record_throttle_at(sent_early, Duration::ZERO, late_answer),
            None,
            "sent at the old rate"
        );
        assert_eq!(limit.current(), 6);

        let sent_during_penalty = first_cut + Duration::from_secs(1);
        assert_eq!(
            limit.record_throttle_at(sent_during_penalty, Duration::ZERO, late_answer),
            None,
            "sent while the penalty was still running"
        );
        assert!(limit
            .record_throttle_at(after_the_penalty, Duration::ZERO, after_the_penalty)
            .is_some());
    }

    #[test]
    fn a_wait_and_its_staggered_release_are_one_cut() {
        let limit = AdaptiveDownloadLimit::new(32);
        let sent = Instant::now();
        let wait = Duration::from_secs(30);
        limit.record_throttle_at(sent, wait, sent).expect("cut");
        for released_after in [30, 31, 32, 33, 35, 39] {
            let released = sent + Duration::from_secs(released_after);
            assert_eq!(
                limit.record_throttle_at(released, wait, released),
                None,
                "still the same penalty at {released_after} s"
            );
        }
        assert_eq!(limit.current(), 6);
    }

    #[test]
    fn after_a_429_it_climbs_back_slowly_to_80_percent_then_probes() {
        let limit = AdaptiveDownloadLimit::new(32);
        let mut now = Instant::now();
        // Climb to 20 parallel at the first-run pace, then get refused there.
        for _ in 0..12 {
            now += RAISE_INTERVAL;
            limit.record_success_at(now);
        }
        assert_eq!(limit.current(), 20);
        limit
            .record_throttle_at(now, Duration::ZERO, now)
            .expect("cut");
        assert_eq!(limit.current(), 15);

        // After a cut, 10 s is no longer enough for a step.
        now += RAISE_INTERVAL;
        assert_eq!(
            limit.record_success_at(now),
            None,
            "one step a minute after a cut"
        );
        now += PROBE_INTERVAL;
        let change = limit.record_success_at(now).expect("a step after a minute");
        assert_eq!(
            (change.from, change.to),
            (15, 16),
            "the settle point, 80% of 20"
        );

        // Above it, the next step is a small probe a minute later.
        now += PROBE_INTERVAL;
        let change = limit.record_success_at(now).expect("probe");
        assert_eq!((change.from, change.to), (16, 17));
    }

    #[test]
    fn never_below_the_floor() {
        let limit = AdaptiveDownloadLimit::new(32);
        let mut now = Instant::now();
        for _ in 0..10 {
            let sent = now + CUT_GRACE + Duration::from_millis(1);
            now = sent + Duration::from_millis(1);
            limit.record_throttle_at(sent, Duration::ZERO, now);
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
