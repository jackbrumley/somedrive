use super::*;

fn has_violation(status: &SyncRuntimeAccountStatus, violation: &str) -> bool {
    status
        .consistency
        .violations
        .iter()
        .any(|value| value == violation)
}

#[test]
fn marks_download_materialization_gap_as_consistency_violation() {
    let mut status = SyncRuntimeAccountStatus::new("sync-runtime-test-download-gap");
    status.phase = "applying_remote".to_string();
    status.planner_need_download_total = 5;
    status.remote_download_planned_total = 0;

    recompute_authority_fields(&mut status);

    assert!(has_violation(
        &status,
        "planner_download_actions_without_materialized_jobs"
    ));
    assert!(!status.consistency.ok);
}

#[test]
fn marks_upload_materialization_gap_as_consistency_violation() {
    let mut status = SyncRuntimeAccountStatus::new("sync-runtime-test-upload-gap");
    status.phase = "applying_local".to_string();
    status.planner_need_upload_total = 3;
    status.upload_planned_total = 0;

    recompute_authority_fields(&mut status);

    assert!(has_violation(
        &status,
        "planner_upload_actions_without_materialized_jobs"
    ));
    assert!(!status.consistency.ok);
}

#[test]
fn materialized_planner_actions_keep_consistency_ok() {
    let mut status = SyncRuntimeAccountStatus::new("sync-runtime-test-consistent");
    status.phase = "applying_remote".to_string();
    status.planner_need_download_total = 4;
    status.remote_download_planned_total = 4;

    recompute_authority_fields(&mut status);

    assert!(!has_violation(
        &status,
        "planner_download_actions_without_materialized_jobs"
    ));
    assert!(status.consistency.ok);
}

#[test]
fn download_overcommit_sets_consistency_violation() {
    let mut status = SyncRuntimeAccountStatus::new("sync-runtime-test-download-overcommit");
    status.remote_download_planned_total = 10;
    status.remote_download_completed_total = 7;
    status.remote_download_failed_total = 2;
    status.remote_download_in_flight = 3;

    recompute_authority_fields(&mut status);

    assert!(has_violation(&status, "download_lane_overcommitted"));
    assert!(!status.consistency.ok);
}

#[test]
fn in_flight_counter_without_active_rows_sets_consistency_violation() {
    let mut status = SyncRuntimeAccountStatus::new("sync-runtime-test-inflight-no-rows");
    status.remote_download_in_flight = 1;
    status.upload_in_flight = 1;

    recompute_authority_fields(&mut status);

    assert!(has_violation(
        &status,
        "in_flight_counter_without_active_rows"
    ));
    assert!(!status.consistency.ok);
}
