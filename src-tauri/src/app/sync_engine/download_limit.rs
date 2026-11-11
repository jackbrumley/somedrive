use super::*;
use std::collections::VecDeque;
use std::time::Instant;

/// Parallel downloads an account starts with before it has learned how much
/// Microsoft will take.
const DOWNLOAD_LIMIT_START: usize = 8;
/// The limit never falls below this, however often Microsoft throttles.
const DOWNLOAD_LIMIT_FLOOR: usize = 4;
/// Before Microsoft has ever refused, the limits rise by one step per this
/// interval while downloads succeed. Microsoft counts usage over a window of
/// minutes, so its 429 arrives well after the rate has passed what it
/// allows: a slow climb overshoots less.
const RAISE_INTERVAL: Duration = Duration::from_secs(30);
/// Each step raises the start rate by this fraction (and parallel downloads
/// by one).
const START_RATE_STEP: f64 = 0.05;
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
/// A 429 sets the rate to this fraction of the request rate Microsoft
/// refused (measured over `RATE_WINDOW`, not the configured rate), so it
/// runs steadily just under the limit instead of dropping far below it and
/// climbing back.
const SETTLE_FRACTION: f64 = 0.85;
/// Above the settle point the limits only probe upwards: one small step per
/// interval, in case Microsoft now allows more.
const PROBE_INTERVAL: Duration = Duration::from_secs(60);
const PROBE_STEP: f64 = 0.02;
/// For this long after a refusal, probing stops at `CEILING_HOLD_FRACTION`
/// of the refused rate, so the same limit is not met again every few
/// minutes. After it, probing may pass the old ceiling (Microsoft's limits
/// move with the load on the tenant).
const CEILING_HOLD: Duration = Duration::from_secs(30 * 60);
const CEILING_HOLD_FRACTION: f64 = 0.95;
/// The request rate is measured over this window.
const RATE_WINDOW: Duration = Duration::from_secs(60);
/// Measuring restarts after each penalty (requests held by Microsoft's wait
/// say nothing about the rate it refused). A refusal with less clean
/// measurement than this takes the configured rate as the refused one.
const MIN_MEASURE_SPAN: Duration = Duration::from_secs(20);
/// After a cut, Microsoft often refuses the requests released after its
/// wait for a while longer: the same penalty, not a verdict on the new rate.
/// A refusal is a new cut only after this many transfers have succeeded
/// since the last one.
const MIN_SUCCESSES_FOR_NEW_CUT: usize = 20;
/// Microsoft's `RateLimit-*` headers (sent once most of a quota is used)
/// ease the start rate by this factor, at most once per `PRESSURE_INTERVAL`
/// and without stopping anything, so the 429 they warn of never comes.
const PRESSURE_EASE: f64 = 0.9;
const PRESSURE_INTERVAL: Duration = Duration::from_secs(30);
/// Files at least this big take the large-file lane (see
/// `large_download_slots`).
pub(crate) const LARGE_DOWNLOAD_BYTES: u64 = 8 * 1024 * 1024;

/// What an account has learned about Microsoft's limit, kept between cycles
/// and restarts (`download_rate_store`) so each cycle starts at the settled
/// rate instead of climbing into the limit again.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LearnedDownloadRate {
    pub(crate) parallel: usize,
    pub(crate) starts_per_second: f64,
    /// Parallel downloads and request rate when Microsoft last refused.
    pub(crate) ceiling: Option<(usize, f64)>,
}

/// How fast one account transfers: how many downloads run at once, and how
/// many may start per second. Before the first refusal both creep up one
/// small step per `RAISE_INTERVAL` while downloads succeed. A 429 sets them
/// to `SETTLE_FRACTION` of the request rate actually measured when Microsoft
/// refused; above that they only probe upwards slowly, and for
/// `CEILING_HOLD` not past `CEILING_HOLD_FRACTION` of the refused rate.
/// Microsoft's early-warning headers ease the rate before a refusal comes.
/// One throttle episode is one cut (`CUT_GRACE`), never a cascade.
pub(crate) struct AdaptiveDownloadLimit {
    maximum: usize,
    state: std::sync::Mutex<AdaptiveDownloadLimitState>,
}

struct AdaptiveDownloadLimitState {
    current: usize,
    starts_per_second: f64,
    start_permits: f64,
    permits_refilled_at: Instant,
    last_change_at: Instant,
    /// 429s for requests started before this belong to the last cut.
    cut_covers_until: Option<Instant>,
    /// Parallel downloads and request rate when Microsoft last refused (or
    /// warned).
    ceiling: Option<(usize, f64)>,
    /// The highest start rate allowed until `hold_until`: 95% of a refused
    /// rate, or the eased rate after an early warning. Set only by a refusal
    /// or warning in this cycle, never by a learned ceiling, so a new cycle
    /// can always climb again.
    hold_rate: Option<f64>,
    hold_until: Option<Instant>,
    /// Transfers that succeeded since the last cut; `None` before the
    /// cycle's first cut.
    successes_since_cut: Option<usize>,
    /// Requests started within the last `RATE_WINDOW`.
    request_starts: VecDeque<Instant>,
    measuring_since: Instant,
    last_pressure_at: Option<Instant>,
}

/// A change to the limits, for the caller to log and save.
#[derive(Debug, PartialEq)]
pub(crate) struct DownloadLimitChange {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) starts_per_second: f64,
}

impl AdaptiveDownloadLimit {
    /// Starts at `learned` (clamped to `maximum`) when the account has
    /// learned a rate, otherwise at the first-run defaults.
    pub(crate) fn new(maximum: usize, learned: Option<LearnedDownloadRate>) -> Self {
        Self::new_at(maximum, learned, Instant::now())
    }

    fn new_at(maximum: usize, learned: Option<LearnedDownloadRate>, now: Instant) -> Self {
        let maximum = maximum.max(1);
        let floor = DOWNLOAD_LIMIT_FLOOR.min(maximum);
        let (current, starts_per_second, ceiling) = match learned {
            Some(learned) => (
                learned.parallel.clamp(floor, maximum),
                learned
                    .starts_per_second
                    .clamp(START_RATE_FLOOR, START_RATE_CEILING),
                // A ceiling below the floor can only come from a measuring
                // fault; it would hold the account at the floor.
                learned
                    .ceiling
                    .filter(|(_, rate)| *rate >= START_RATE_FLOOR),
            ),
            None => (DOWNLOAD_LIMIT_START.min(maximum), START_RATE_INITIAL, None),
        };
        Self {
            maximum,
            state: std::sync::Mutex::new(AdaptiveDownloadLimitState {
                current,
                starts_per_second,
                start_permits: starts_per_second,
                permits_refilled_at: now,
                last_change_at: now,
                cut_covers_until: None,
                // A learned ceiling sets the settle point and the slow probe
                // pace above it; it is not held.
                ceiling,
                hold_rate: None,
                hold_until: None,
                successes_since_cut: None,
                request_starts: VecDeque::new(),
                measuring_since: now,
                last_pressure_at: None,
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

    /// The rate to keep for the next cycle.
    pub(crate) fn learned(&self) -> Option<LearnedDownloadRate> {
        let state = self.state.lock().ok()?;
        Some(LearnedDownloadRate {
            parallel: state.current,
            starts_per_second: state.starts_per_second,
            ceiling: state.ceiling,
        })
    }

    /// Slots kept for files of `LARGE_DOWNLOAD_BYTES` or more: a quarter of
    /// the limit (at least one), so a few big files cannot hold every slot
    /// while thousands of small ones wait.
    pub(crate) fn large_download_slots(&self) -> usize {
        (self.current() / 4).max(1)
    }

    /// Counts a Graph request starting now, for the measured request rate.
    pub(crate) fn record_request_start(&self) {
        self.record_request_start_at(Instant::now());
    }

    fn record_request_start_at(&self, now: Instant) {
        if let Ok(mut state) = self.state.lock() {
            state.request_starts.push_back(now);
            prune_request_starts(&mut state, now);
        }
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

    /// Records a finished transfer; returns the change when it raises the
    /// limits (see the type's description for the pace).
    pub(crate) fn record_success(&self) -> Option<DownloadLimitChange> {
        self.record_success_at(Instant::now())
    }

    fn record_success_at(&self, now: Instant) -> Option<DownloadLimitChange> {
        let mut state = self.state.lock().ok()?;
        if let Some(successes) = state.successes_since_cut.as_mut() {
            *successes += 1;
        }
        let interval = if state.ceiling.is_some() {
            PROBE_INTERVAL
        } else {
            RAISE_INTERVAL
        };
        if now.saturating_duration_since(state.last_change_at) < interval {
            return None;
        }
        let (mut parallel_cap, mut rate_cap) = (self.maximum, START_RATE_CEILING);
        let mut probing = false;
        if let Some((ceiling_parallel, ceiling_rate)) = state.ceiling {
            let settle_rate = ceiling_rate * SETTLE_FRACTION;
            probing = state.starts_per_second >= settle_rate;
            if state.hold_until.is_some_and(|until| now < until) {
                parallel_cap = parallel_cap.min(ceiling_parallel.max(state.current));
                let hold_rate = state
                    .hold_rate
                    .unwrap_or(ceiling_rate * CEILING_HOLD_FRACTION);
                rate_cap = rate_cap.min(hold_rate.max(START_RATE_FLOOR));
            }
            if !probing {
                // Climbing back: stop at the settle point, never past it.
                rate_cap = rate_cap.min(settle_rate);
            }
        }
        let rate_step = if probing { PROBE_STEP } else { START_RATE_STEP };
        let next_parallel = (state.current + 1).min(parallel_cap).max(state.current);
        let next_rate = (state.starts_per_second * (1.0 + rate_step))
            .min(rate_cap)
            .max(state.starts_per_second);
        if next_parallel == state.current && next_rate <= state.starts_per_second {
            return None;
        }
        let from = state.current;
        state.current = next_parallel;
        state.starts_per_second = next_rate;
        state.last_change_at = now;
        Some(DownloadLimitChange {
            from,
            to: state.current,
            starts_per_second: state.starts_per_second,
        })
    }

    /// Records a 429 for a request that started at `request_started_at`,
    /// answered with "wait `retry_after`". Sets the limits to
    /// `SETTLE_FRACTION` of the measured request rate, unless the refusal
    /// belongs to the last cut's penalty (the request started before it
    /// ended, or too few transfers have succeeded since). Returns the change
    /// when it lowered them.
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
        let started_in_penalty = state
            .cut_covers_until
            .is_some_and(|covered| request_started_at <= covered);
        let same_episode = state
            .successes_since_cut
            .is_some_and(|successes| successes < MIN_SUCCESSES_FOR_NEW_CUT);
        if started_in_penalty || same_episode {
            return None;
        }
        // The configured rate can be above what actually ran (too few jobs
        // ready, all slots busy, or a cycle that just started into a window
        // an earlier one filled): the measured rate is what Microsoft saw,
        // once enough of it has been measured since the last penalty.
        let measured_long_enough =
            now.saturating_duration_since(state.measuring_since) >= MIN_MEASURE_SPAN;
        let measured = measured_request_rate(&mut state, now);
        let refused_rate = if measured_long_enough && measured > 0.0 {
            measured.min(state.starts_per_second)
        } else {
            state.starts_per_second
        };
        state.cut_covers_until = Some(now + retry_after + CUT_GRACE);
        state.last_change_at = now + retry_after;
        state.ceiling = Some((state.current, refused_rate));
        state.hold_until = Some(now + CEILING_HOLD);
        state.hold_rate = Some((refused_rate * CEILING_HOLD_FRACTION).max(START_RATE_FLOOR));
        state.successes_since_cut = Some(0);
        state.request_starts.clear();
        state.measuring_since = now + retry_after;
        let from = state.current;
        state.current =
            ((state.current as f64 * SETTLE_FRACTION).floor() as usize).max(self.floor());
        state.starts_per_second = (refused_rate * SETTLE_FRACTION).max(START_RATE_FLOOR);
        state.start_permits = state.start_permits.min(state.starts_per_second);
        Some(DownloadLimitChange {
            from,
            to: state.current,
            starts_per_second: state.starts_per_second,
        })
    }

    /// Records Microsoft's early warning (`RateLimit-*` headers: most of a
    /// quota is used): eases the start rate a little, at most once per
    /// `PRESSURE_INTERVAL`, and holds it at the eased rate for
    /// `CEILING_HOLD` (a warning means the rate before it was already too
    /// close). Nothing stops. Returns the change when it eased.
    pub(crate) fn record_pressure(&self) -> Option<DownloadLimitChange> {
        self.record_pressure_at(Instant::now())
    }

    fn record_pressure_at(&self, now: Instant) -> Option<DownloadLimitChange> {
        let mut state = self.state.lock().ok()?;
        let recently_eased = state
            .last_pressure_at
            .is_some_and(|eased| now.saturating_duration_since(eased) < PRESSURE_INTERVAL);
        let penalty_running = state.cut_covers_until.is_some_and(|covered| now <= covered);
        if recently_eased || penalty_running {
            return None;
        }
        let before = state.starts_per_second;
        let eased = (before * PRESSURE_EASE).max(START_RATE_FLOOR);
        state.last_pressure_at = Some(now);
        if eased >= before {
            return None;
        }
        state.starts_per_second = eased;
        state.start_permits = state.start_permits.min(eased);
        state.last_change_at = now;
        state.hold_rate = Some(eased);
        state.hold_until = Some(now + CEILING_HOLD);
        if state
            .ceiling
            .is_none_or(|(_, ceiling_rate)| ceiling_rate > before)
        {
            state.ceiling = Some((state.current, before));
        }
        Some(DownloadLimitChange {
            from: state.current,
            to: state.current,
            starts_per_second: state.starts_per_second,
        })
    }
}

fn prune_request_starts(state: &mut AdaptiveDownloadLimitState, now: Instant) {
    while state
        .request_starts
        .front()
        .is_some_and(|started| now.saturating_duration_since(*started) > RATE_WINDOW)
    {
        state.request_starts.pop_front();
    }
}

/// Requests per second over the last `RATE_WINDOW` (or since the cycle
/// started, when that is shorter).
fn measured_request_rate(state: &mut AdaptiveDownloadLimitState, now: Instant) -> f64 {
    prune_request_starts(state, now);
    let span = now
        .saturating_duration_since(state.measuring_since)
        .min(RATE_WINDOW)
        .as_secs_f64()
        .max(1.0);
    state.request_starts.len() as f64 / span
}

#[cfg(test)]
#[path = "download_limit_tests.rs"]
mod adaptive_limit_tests;
