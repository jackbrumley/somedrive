//! Tests for the adaptive transfer limit.
use super::*;

fn learned(parallel: usize, rate: f64, ceiling: Option<(usize, f64)>) -> LearnedDownloadRate {
    LearnedDownloadRate {
        parallel,
        starts_per_second: rate,
        ceiling,
    }
}

#[test]
fn starts_at_eight_or_the_setting_when_lower() {
    assert_eq!(AdaptiveDownloadLimit::new(12, None).current(), 8);
    assert_eq!(AdaptiveDownloadLimit::new(4, None).current(), 4);
}

#[test]
fn starts_from_the_learned_rate_within_the_setting() {
    let now = Instant::now();
    let limit = AdaptiveDownloadLimit::new_at(32, Some(learned(12, 15.0, Some((16, 20.0)))), now);
    assert_eq!(limit.current(), 12);
    assert_eq!(limit.take_start_permits_at(100, now), 15);
    assert_eq!(
        limit.learned(),
        Some(learned(12, 15.0, Some((16, 20.0)))),
        "kept as learned"
    );
    let smaller = AdaptiveDownloadLimit::new(6, Some(learned(12, 99.0, None)));
    assert_eq!(smaller.current(), 6, "never above the setting");
    assert_eq!(
        smaller.learned().map(|rate| rate.starts_per_second),
        Some(START_RATE_CEILING)
    );
}

#[test]
fn a_learned_ceiling_is_not_held_and_one_below_the_floor_is_ignored() {
    let mut now = Instant::now();
    let limit = AdaptiveDownloadLimit::new_at(32, Some(learned(10, 8.0, Some((12, 9.0)))), now);
    now += PROBE_INTERVAL;
    let change = limit
        .record_success_at(now)
        .expect("probes again in a new cycle");
    assert!(change.starts_per_second > 8.0);

    let faulty = AdaptiveDownloadLimit::new(32, Some(learned(6, 2.0, Some((6, 0.1)))));
    assert_eq!(
        faulty.learned().and_then(|rate| rate.ceiling),
        None,
        "a ceiling below the floor is dropped"
    );
}

#[test]
fn rises_one_small_step_per_interval_and_never_above_the_setting() {
    let start = Instant::now();
    let limit = AdaptiveDownloadLimit::new_at(9, None, start);
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
    assert!((change.starts_per_second - START_RATE_INITIAL * 1.05).abs() < 1e-9);
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
fn a_429_settles_at_85_percent_of_the_measured_rate() {
    let start = Instant::now();
    let limit = AdaptiveDownloadLimit::new_at(32, Some(learned(20, 20.0, None)), start);
    // Configured for 20 a second, but only 10 a second actually ran.
    for tenth in 0..600 {
        limit.record_request_start_at(start + Duration::from_millis(100 * tenth));
    }
    let refused_at = start + RATE_WINDOW;
    let change = limit
        .record_throttle_at(refused_at, Duration::ZERO, refused_at)
        .expect("cut");
    assert_eq!((change.from, change.to), (20, 17));
    assert!(
        (change.starts_per_second - 8.5).abs() < 0.1,
        "85% of the measured 10 a second, not of the configured 20: {}",
        change.starts_per_second
    );
}

#[test]
fn a_cut_restarts_the_interval() {
    let sent = Instant::now();
    let limit = AdaptiveDownloadLimit::new_at(32, None, sent);
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
    let sent_early = Instant::now();
    let limit = AdaptiveDownloadLimit::new_at(32, None, sent_early);
    let first_cut = sent_early + Duration::from_millis(10);
    let change = limit
        .record_throttle_at(sent_early, Duration::ZERO, first_cut)
        .expect("cut");
    assert_eq!((change.from, change.to), (8, 6), "85%, rounded down");

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
    for _ in 0..MIN_SUCCESSES_FOR_NEW_CUT {
        limit.record_success_at(late_answer);
    }
    assert!(limit
        .record_throttle_at(after_the_penalty, Duration::ZERO, after_the_penalty)
        .is_some());
}

#[test]
fn a_wait_and_its_staggered_release_are_one_cut() {
    let sent = Instant::now();
    let limit = AdaptiveDownloadLimit::new_at(32, None, sent);
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
fn after_a_429_it_probes_slowly_and_holds_under_the_old_ceiling() {
    let mut now = Instant::now();
    let limit = AdaptiveDownloadLimit::new_at(32, Some(learned(20, 20.0, None)), now);
    limit
        .record_throttle_at(now, Duration::ZERO, now)
        .expect("cut");
    assert_eq!(limit.current(), 17);

    now += RAISE_INTERVAL;
    assert_eq!(
        limit.record_success_at(now),
        None,
        "one step a minute after a cut"
    );
    now += PROBE_INTERVAL;
    let change = limit
        .record_success_at(now)
        .expect("a probe after a minute");
    assert_eq!((change.from, change.to), (17, 18));
    assert!((change.starts_per_second - 17.0 * 1.02).abs() < 1e-9);

    for _ in 0..20 {
        now += PROBE_INTERVAL;
        limit.record_success_at(now);
    }
    let held = limit.learned().expect("rate");
    assert_eq!(held.parallel, 20, "not past the refused parallel count");
    assert!(
        (held.starts_per_second - 19.0).abs() < 1e-9,
        "held at 95% of the refused rate for half an hour: {}",
        held.starts_per_second
    );

    now += CEILING_HOLD;
    limit.record_success_at(now).expect("probing resumes");
    assert!(limit.learned().expect("rate").starts_per_second > 19.0);
}

#[test]
fn never_below_the_floor() {
    let mut now = Instant::now();
    let limit = AdaptiveDownloadLimit::new_at(32, None, now);
    for _ in 0..10 {
        for _ in 0..MIN_SUCCESSES_FOR_NEW_CUT {
            limit.record_success_at(now);
        }
        let sent = now + CUT_GRACE + Duration::from_millis(1);
        now = sent + Duration::from_millis(1);
        limit.record_throttle_at(sent, Duration::ZERO, now);
    }
    assert_eq!(limit.current(), DOWNLOAD_LIMIT_FLOOR);
    assert!(limit.learned().expect("rate").starts_per_second >= START_RATE_FLOOR);
}

#[test]
fn an_early_warning_eases_without_a_stop_and_at_most_every_30_seconds() {
    let now = Instant::now();
    let limit = AdaptiveDownloadLimit::new_at(32, Some(learned(16, 20.0, None)), now);
    let change = limit.record_pressure_at(now).expect("eased");
    assert_eq!((change.from, change.to), (16, 16), "parallel unchanged");
    assert!((change.starts_per_second - 18.0).abs() < 1e-9);
    assert_eq!(
        limit.take_start_permits_at(100, now),
        18,
        "starts continue at once"
    );
    assert_eq!(
        limit.record_pressure_at(now + Duration::from_secs(10)),
        None,
        "one ease per interval"
    );
    assert!(limit.record_pressure_at(now + PRESSURE_INTERVAL).is_some());
    assert_eq!(
        limit.learned().and_then(|rate| rate.ceiling),
        Some((16, 18.0)),
        "a warning still coming at 18 a second lowers the ceiling to 18"
    );
}

#[test]
fn refusals_during_a_penalty_are_one_episode_until_transfers_succeed() {
    let start = Instant::now();
    let limit = AdaptiveDownloadLimit::new_at(32, Some(learned(20, 20.0, None)), start);
    limit
        .record_throttle_at(start, Duration::from_secs(30), start)
        .expect("first cut");
    // Microsoft refuses the probes released after its wait.
    let probe = start + Duration::from_secs(45);
    assert_eq!(
        limit.record_throttle_at(probe, Duration::from_secs(30), probe),
        None,
        "still the same penalty: nothing at the new rate has succeeded yet"
    );
    for second in 0..MIN_SUCCESSES_FOR_NEW_CUT as u64 {
        limit.record_success_at(probe + Duration::from_secs(30 + second));
    }
    let later = probe + Duration::from_secs(120);
    assert!(
        limit
            .record_throttle_at(later, Duration::ZERO, later)
            .is_some(),
        "refused again after transfers at the new rate succeeded: a new cut"
    );
}

#[test]
fn a_held_period_is_not_taken_as_the_refused_rate() {
    let start = Instant::now();
    let limit = AdaptiveDownloadLimit::new_at(32, Some(learned(20, 20.0, None)), start);
    for tenth in 0..600 {
        limit.record_request_start_at(start + Duration::from_millis(100 * tenth));
    }
    let first_cut = start + RATE_WINDOW;
    let wait = Duration::from_secs(30);
    let change = limit
        .record_throttle_at(first_cut, wait, first_cut)
        .expect("cut");
    assert!((change.starts_per_second - 8.5).abs() < 0.1);
    // Transfers resume after the wait; Microsoft refuses again 15 s later
    // (after the cut's grace, before 20 s of clean measurement).
    let resumed = first_cut + wait;
    for second in 0..MIN_SUCCESSES_FOR_NEW_CUT as u64 {
        limit.record_request_start_at(resumed + Duration::from_millis(500 * second));
        limit.record_success_at(resumed + Duration::from_millis(500 * second));
    }
    let refused = resumed + Duration::from_secs(15);
    let change = limit
        .record_throttle_at(refused, wait, refused)
        .expect("second cut");
    assert!(
        (change.starts_per_second - 8.5 * SETTLE_FRACTION).abs() < 0.1,
        "85% of the configured rate, not of the few requests the wait let through: {}",
        change.starts_per_second
    );
}

#[test]
fn after_an_early_warning_it_holds_at_the_eased_rate() {
    let mut now = Instant::now();
    let limit = AdaptiveDownloadLimit::new_at(32, Some(learned(16, 20.0, None)), now);
    limit.record_pressure_at(now).expect("eased");
    for _ in 0..20 {
        now += PROBE_INTERVAL;
        limit.record_success_at(now);
    }
    assert!(
        (limit.learned().expect("rate").starts_per_second - 18.0).abs() < 1e-9,
        "no probing back towards the warned rate during the hold"
    );
}

/// Microsoft modelled as a quota of `quota` requests a second averaged
/// over a minute, answering 429 with "wait 30 s" when it is exceeded.
#[test]
fn a_steady_quota_is_met_once_and_then_held_without_stops() {
    let quota = 14.0;
    let start = Instant::now();
    let limit = AdaptiveDownloadLimit::new_at(32, None, start);
    let mut window: VecDeque<Instant> = VecDeque::new();
    let mut refusals = Vec::new();
    let mut paused_until = start;
    let half_hour = 30 * 60;
    for second in 0..half_hour {
        let now = start + Duration::from_secs(second);
        if now < paused_until {
            continue;
        }
        while window
            .front()
            .is_some_and(|sent| now.saturating_duration_since(*sent) > RATE_WINDOW)
        {
            window.pop_front();
        }
        let granted = limit.take_start_permits_at(usize::MAX, now);
        for _ in 0..granted {
            limit.record_request_start_at(now);
            window.push_back(now);
        }
        if window.len() as f64 > quota * RATE_WINDOW.as_secs_f64() {
            let wait = Duration::from_secs(30);
            limit.record_throttle_at(now, wait, now);
            refusals.push(second);
            paused_until = now + wait;
        } else {
            limit.record_success_at(now);
        }
    }
    assert_eq!(
        refusals.len(),
        1,
        "the limit is met once, then the rate holds under it: {refusals:?}"
    );
    let settled = limit.learned().expect("rate").starts_per_second;
    assert!(
        settled >= quota * 0.8 && settled < quota,
        "settled just under the quota: {settled}"
    );
}

#[test]
fn starts_are_paced_to_the_start_rate() {
    let now = Instant::now();
    let limit = AdaptiveDownloadLimit::new_at(32, None, now);
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
    assert_eq!(
        AdaptiveDownloadLimit::new(32, None).large_download_slots(),
        2
    );
    assert_eq!(
        AdaptiveDownloadLimit::new(2, None).large_download_slots(),
        1
    );
}
