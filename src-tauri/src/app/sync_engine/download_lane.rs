use super::*;

pub(crate) fn sync_download_counters_from_db(
    graph: &GraphContext,
) -> Result<DownloadJobCounters, String> {
    let counters = read_download_job_counters(&graph.profile_id)?;
    runtime_set_remote_download_counters(
        &graph.sync_runtime,
        &graph.profile_id,
        counters.planned_total,
        counters.completed,
        counters.failed_terminal,
        counters.in_progress,
        counters.retry_waiting,
    );
    Ok(counters)
}

pub(crate) async fn dispatch_claimed_download_jobs(
    graph: &GraphContext,
    sync_root: &Path,
    download_tx: &mpsc::Sender<RemoteDownloadJob>,
    available_slots: usize,
    pending_download_count: usize,
    remaining_download_jobs: usize,
) -> Result<usize, String> {
    if available_slots == 0 {
        return Ok(0);
    }

    let claimed = claim_download_jobs(&graph.profile_id, &graph.cycle_id, available_slots)?;
    if claimed.is_empty() {
        return Ok(0);
    }

    let mut dispatched: usize = 0;
    for claimed_job in claimed {
        // The version to download is captured now, so the baseline records
        // exactly what was fetched even if the cloud changes meanwhile.
        let Some(row) = read_sync_file_row(&graph.profile_id, &claimed_job.path)?.filter(|row| {
            row.cloud_present && row.remote_item_id.as_deref() == Some(claimed_job.item_id.as_str())
        }) else {
            // The plan moved on (the item was deleted or moved): drop the job.
            mark_download_job_done(&graph.profile_id, claimed_job.job_id, true)?;
            continue;
        };
        let local_abs = sync_root.join(path_to_local(&claimed_job.path));
        let job = RemoteDownloadJob {
            job_id: claimed_job.job_id,
            item_id: claimed_job.item_id,
            path: claimed_job.path,
            local_abs,
            version: row.cloud_version(),
            cloud_mtime: row.cloud_mtime,
        };

        send_download_job_with_logging(
            graph,
            download_tx,
            job,
            pending_download_count + dispatched,
            remaining_download_jobs,
        )
        .await?;
        dispatched += 1;
    }
    Ok(dispatched)
}

pub(crate) async fn send_download_job_with_logging(
    graph: &GraphContext,
    download_tx: &mpsc::Sender<RemoteDownloadJob>,
    job: RemoteDownloadJob,
    pending_download_count: usize,
    remaining_download_jobs: usize,
) -> Result<(), String> {
    let item_id = job.item_id.clone();
    let path = job.path.clone();
    let enqueue_started_at = std::time::Instant::now();
    let mut wait_logged = false;
    let send_future = download_tx.send(job);
    tokio::pin!(send_future);
    loop {
        tokio::select! {
            send_result = &mut send_future => {
                if wait_logged {
                    log::info!(
                        "{} [cycle:{}] DOWNLOAD_ENQUEUE_WAIT_DONE item_id={} path={} wait_s={} pending_downloads={} remaining_download_jobs={}",
                        graph.account_prefix,
                        graph.cycle_id,
                        item_id,
                        path,
                        enqueue_started_at.elapsed().as_secs(),
                        pending_download_count,
                        remaining_download_jobs
                    );
                }
                return send_result.map_err(|_| "Remote download queue closed unexpectedly".to_string());
            }
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                wait_logged = true;
                log::warn!(
                    "{} [cycle:{}] DOWNLOAD_ENQUEUE_WAITING item_id={} path={} wait_s={} pending_downloads={} remaining_download_jobs={}",
                    graph.account_prefix,
                    graph.cycle_id,
                    item_id,
                    path,
                    enqueue_started_at.elapsed().as_secs(),
                    pending_download_count,
                    remaining_download_jobs
                );
            }
        }
    }
}

/// How long a download that got 404/410 waits before it is tried again; a
/// delta page normally confirms the delete well before then.
pub(crate) const DOWNLOAD_NOT_FOUND_RETRY_DELAY: Duration = Duration::from_secs(600);

pub(crate) fn apply_remote_download_result(
    graph: &GraphContext,
    stats: &mut SyncCycleStats,
    result: RemoteDownloadResult,
) -> Result<(), String> {
    match result.status {
        RemoteDownloadResultStatus::DeferredRetry => {
            log::info!(
                "{} [cycle:{}] REMOTE_DOWNLOAD_JOB_RETRY_DEFERRED path={} id={}",
                graph.account_prefix,
                graph.cycle_id,
                result.path,
                result.item_id
            );
        }
        RemoteDownloadResultStatus::Cancelled => {
            mark_download_job_retry_wait(
                &graph.profile_id,
                result.job_id,
                "Download cancelled due to pause; retry scheduled on resume",
                Duration::from_secs(1),
            )?;
            log::info!(
                "{} [cycle:{}] REMOTE_DOWNLOAD_JOB_CANCELLED_RETRY_WAIT path={} id={}",
                graph.account_prefix,
                graph.cycle_id,
                result.path,
                result.item_id
            );
        }
        RemoteDownloadResultStatus::Failed(error_text)
            if !runtime_profile_auth_ready(&graph.sync_runtime, &graph.profile_id) =>
        {
            // The sign-in expired mid-download: not the file's fault. It
            // runs again once the account is signed in.
            mark_download_job_retry_wait(
                &graph.profile_id,
                result.job_id,
                "Waiting for sign-in",
                Duration::from_secs(1),
            )?;
            log::info!(
                "{} [cycle:{}] REMOTE_DOWNLOAD_REQUEUED_FOR_SIGN_IN path={} error={}",
                graph.account_prefix,
                graph.cycle_id,
                result.path,
                error_text
            );
        }
        RemoteDownloadResultStatus::Failed(error_text) => {
            mark_download_job_failed(&graph.profile_id, result.job_id, &error_text)?;
            runtime_record_remote_download_failed(
                &graph.sync_runtime,
                &graph.profile_id,
                &result.item_id,
            );
            log::warn!(
                "{} [cycle:{}] REMOTE_DOWNLOAD_JOB_FAILED path={} id={} error={}",
                graph.account_prefix,
                graph.cycle_id,
                result.path,
                result.item_id,
                error_text
            );
        }
        RemoteDownloadResultStatus::Success(RemoteDownloadOutcome::Downloaded(entry)) => {
            commit_baseline(&graph.profile_id, &result.path, &result.version, &entry)?;
            mark_download_job_done(&graph.profile_id, result.job_id, false)?;
            runtime_record_remote_download_completed(
                &graph.sync_runtime,
                &graph.profile_id,
                &result.item_id,
            );
            stats.downloaded_files += 1;
        }
        RemoteDownloadResultStatus::Success(RemoteDownloadOutcome::HeldAsConflict) => {
            mark_download_job_done(&graph.profile_id, result.job_id, true)?;
            stats.conflicts_held += 1;
        }
        RemoteDownloadResultStatus::Success(RemoteDownloadOutcome::NotFoundOnCloud) => {
            mark_download_job_retry_wait(
                &graph.profile_id,
                result.job_id,
                "OneDrive could not find this file; waiting for its change feed",
                DOWNLOAD_NOT_FOUND_RETRY_DELAY,
            )?;
            log::warn!(
                "{} [cycle:{}] REMOTE_ITEM_NOT_FOUND path={} id={} retry_in_s={}",
                graph.account_prefix,
                graph.cycle_id,
                result.path,
                result.item_id,
                DOWNLOAD_NOT_FOUND_RETRY_DELAY.as_secs()
            );
        }
    }
    Ok(())
}
