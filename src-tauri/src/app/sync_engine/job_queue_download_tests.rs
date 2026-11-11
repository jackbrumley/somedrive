//! Tests for the download queue: size lanes and following the plan.
use super::*;

fn test_profile_id(label: &str) -> String {
    format!(
        "download-queue-test-{label}-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    )
}

fn clear_profile_rows(profile_id: &str) {
    let connection = open_sync_jobs_connection(profile_id).expect("open sync jobs db");
    for table in ["sync_jobs", "sync_files"] {
        connection
            .execute(
                &format!("DELETE FROM {table} WHERE profile_id = ?1"),
                params![profile_id],
            )
            .expect("clear rows");
    }
}

#[test]
fn large_files_hold_only_their_lane_while_small_files_wait() {
    let profile_id = test_profile_id("download-lanes");
    clear_profile_rows(&profile_id);
    let large = LARGE_DOWNLOAD_BYTES;
    for index in 0..4 {
        insert_queued_download(&profile_id, &format!("big-{index}"), large);
    }
    for index in 0..6 {
        insert_queued_download(&profile_id, &format!("small-{index}"), 10);
    }

    let first = claim_download_jobs(&profile_id, "cycle-1", 4, 1).expect("claim");
    let first_large = first
        .iter()
        .filter(|job| job.item_id.starts_with("big-"))
        .count();
    assert_eq!(first.len(), 4);
    assert_eq!(first_large, 1, "one large slot, the rest go to small files");

    let second = claim_download_jobs(&profile_id, "cycle-1", 4, 1).expect("claim");
    let second_small = second
        .iter()
        .filter(|job| job.item_id.starts_with("small-"))
        .count();
    assert_eq!(
        second_small, 3,
        "small files first while the large lane is full"
    );
    assert_eq!(
        second.len(),
        4,
        "the slot no small file needs goes to a large file"
    );
    let third = claim_download_jobs(&profile_id, "cycle-1", 4, 1).expect("claim");
    assert_eq!(third.len(), 2);
    assert!(third.iter().all(|job| job.item_id.starts_with("big-")));
    clear_profile_rows(&profile_id);
}

/// A queued download job of `size` bytes, straight into the queue.
fn insert_queued_download(profile_id: &str, item_id: &str, size: u64) {
    let connection = open_sync_jobs_connection(profile_id).expect("open");
    connection
        .execute(
            "INSERT INTO sync_jobs (profile_id, direction, item_id, path, remote_size,
                remote_modified_ts, state, run_state, attempt_count, bytes_done,
                created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3, ?4, 1, ?5, ?6, 0, 0, 1, 1)",
            params![
                profile_id,
                DOWNLOAD_JOB_DIRECTION,
                item_id,
                size as i64,
                DOWNLOAD_JOB_STATE_QUEUED,
                JOB_RUN_STATE_IDLE,
            ],
        )
        .expect("insert queued download");
}

#[test]
fn the_download_queue_follows_the_plan_and_rewrites_only_what_changed() {
    let profile_id = test_profile_id("queue-planned");
    clear_profile_rows(&profile_id);
    let connection = open_sync_jobs_connection(&profile_id).expect("open");
    for (item, action) in [("wanted", "download"), ("verified", "verify")] {
        connection
            .execute(
                "INSERT INTO sync_files (profile_id, path, is_dir, remote_item_id, cloud_present,
                    cloud_size, cloud_mtime, desired_action, updated_at)
                 VALUES (?1, ?2, 0, ?2, 1, 100, 5, ?3, 1)",
                params![profile_id, item, action],
            )
            .expect("plan row");
    }
    insert_queued_download(&profile_id, "stale", 10);
    drop(connection);

    assert_eq!(queue_planned_downloads(&profile_id).expect("queue"), 1);
    let connection = open_sync_jobs_connection(&profile_id).expect("open");
    let queued: Vec<String> = connection
        .prepare("SELECT item_id FROM sync_jobs WHERE profile_id = ?1 ORDER BY item_id")
        .expect("prepare")
        .query_map(params![profile_id], |row| row.get(0))
        .expect("query")
        .collect::<rusqlite::Result<_>>()
        .expect("rows");
    assert_eq!(
        queued,
        vec!["wanted".to_string()],
        "the unplanned queued job was dropped"
    );

    connection
        .execute(
            "UPDATE sync_jobs SET updated_at = 7 WHERE profile_id = ?1",
            params![profile_id],
        )
        .expect("mark");
    queue_planned_downloads(&profile_id).expect("queue again");
    let untouched: i64 = connection
        .query_row(
            "SELECT updated_at FROM sync_jobs WHERE profile_id = ?1",
            params![profile_id],
            |row| row.get(0),
        )
        .expect("read");
    assert_eq!(untouched, 7, "an unchanged job is not rewritten");

    connection
        .execute(
            "UPDATE sync_jobs SET state = 'done' WHERE profile_id = ?1",
            params![profile_id],
        )
        .expect("finish");
    queue_planned_downloads(&profile_id).expect("queue after done");
    let state: String = connection
        .query_row(
            "SELECT state FROM sync_jobs WHERE profile_id = ?1",
            params![profile_id],
            |row| row.get(0),
        )
        .expect("read");
    assert_eq!(
        state, DOWNLOAD_JOB_STATE_QUEUED,
        "a finished job the plan still wants is queued again"
    );
    drop(connection);
    clear_profile_rows(&profile_id);
}

#[test]
fn dropping_unplanned_downloads_looks_each_job_up_by_item() {
    let profile_id = test_profile_id("drop-unplanned-plan");
    let connection = open_sync_jobs_connection(&profile_id).expect("open sync jobs db");
    let mut statement = connection
        .prepare(&format!(
            "EXPLAIN QUERY PLAN {DROP_UNPLANNED_DOWNLOADS_SQL}"
        ))
        .expect("prepare query plan");
    let plan: Vec<String> = statement
        .query_map(
            params![
                profile_id,
                PLANNER_ACTION_DOWNLOAD,
                DOWNLOAD_JOB_DIRECTION,
                DOWNLOAD_JOB_STATE_QUEUED,
                DOWNLOAD_JOB_STATE_RETRY_WAIT
            ],
            |row| row.get::<_, String>(3),
        )
        .expect("query plan")
        .collect::<rusqlite::Result<_>>()
        .expect("read plan");
    assert!(
        plan.iter()
            .any(|step| step.contains("idx_sync_files_remote_item")),
        "each queued job is checked by item, not by scanning the plan: {plan:?}"
    );
}
