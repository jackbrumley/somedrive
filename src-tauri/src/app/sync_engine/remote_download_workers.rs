fn spawn_remote_download_workers(
    graph: &GraphContext,
    download_concurrency: usize,
    cycle_cancel_flag: &Arc<AtomicBool>,
    download_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<RemoteDownloadJob>>>,
    download_result_tx: mpsc::Sender<RemoteDownloadResult>,
    active_download_workers: Arc<AtomicUsize>,
    pending_download_count: Arc<AtomicUsize>,
) {
    for worker_index in 0..download_concurrency {
        let worker_graph = graph.clone();
        let worker_cancel = Arc::clone(cycle_cancel_flag);
        let worker_download_rx = Arc::clone(&download_rx);
        let worker_result_tx = download_result_tx.clone();
        let active_worker_count = Arc::clone(&active_download_workers);
        let worker_pending_download_count = Arc::clone(&pending_download_count);
        let task_guard = DownloadWorkerTaskGuard::register();
        tauri::async_runtime::spawn(async move {
            let _task_guard = task_guard;
            active_worker_count.fetch_add(1, Ordering::Relaxed);
            log::info!(
                "{} [cycle:{}] DOWNLOAD_WORKER_STARTED worker_index={} active_workers={}",
                worker_graph.account_prefix,
                worker_graph.cycle_id,
                worker_index,
                active_worker_count.load(Ordering::Relaxed)
            );
            loop {
                let maybe_job = {
                    let mut receiver = worker_download_rx.lock().await;
                    receiver.recv().await
                };

                let Some(job) = maybe_job else {
                    log::info!(
                        "{} [cycle:{}] DOWNLOAD_WORKER_STOP worker_index={} reason=queue_closed",
                        worker_graph.account_prefix,
                        worker_graph.cycle_id,
                        worker_index
                    );
                    break;
                };

                // Jobs already queued when an update starts preparing are
                // handed back untouched (requeued like a pause) rather than
                // started.
                let result = if !sync_accepts_new_work() {
                    RemoteDownloadResult {
                        job_id: job.job_id,
                        remote_entry: job.remote_entry,
                        status: RemoteDownloadResultStatus::Cancelled,
                    }
                } else {
                    execute_remote_download_job(&worker_graph, job, &worker_cancel).await
                };
                let _ = worker_pending_download_count.try_update(
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                    |current| Some(current.saturating_sub(1)),
                );

                if let Err(send_error) = worker_result_tx.send(result).await {
                    let dropped_result = send_error.0;
                    if let Err(error) = mark_download_job_retry_wait(
                        &worker_graph.profile_id,
                        dropped_result.job_id,
                        "Download worker result channel closed; retry scheduled",
                        Duration::from_secs(1),
                    ) {
                        log::warn!(
                            "{} [cycle:{}] DOWNLOAD_WORKER_RESULT_REQUEUE_FAILED worker_index={} job_id={} error={}",
                            worker_graph.account_prefix,
                            worker_graph.cycle_id,
                            worker_index,
                            dropped_result.job_id,
                            error
                        );
                    }
                    log::info!(
                        "{} [cycle:{}] DOWNLOAD_WORKER_STOP worker_index={} reason=result_channel_closed",
                        worker_graph.account_prefix,
                        worker_graph.cycle_id,
                        worker_index
                    );
                    break;
                }
            }
            let remaining = active_worker_count
                .fetch_sub(1, Ordering::Relaxed)
                .saturating_sub(1);
            log::info!(
                "{} [cycle:{}] DOWNLOAD_WORKER_EXITED worker_index={} active_workers={}",
                worker_graph.account_prefix,
                worker_graph.cycle_id,
                worker_index,
                remaining
            );
        });
    }
}

async fn execute_remote_download_job(
    worker_graph: &GraphContext,
    job: RemoteDownloadJob,
    worker_cancel: &Arc<AtomicBool>,
) -> RemoteDownloadResult {
    match download_remote_item_content(
        worker_graph,
        Some(job.job_id),
        &job.item_id,
        &job.path,
        Some(job.remote_entry.size),
        &job.local_abs,
        worker_cancel,
    )
    .await
    {
        Ok(outcome) => RemoteDownloadResult {
            job_id: job.job_id,
            remote_entry: job.remote_entry,
            status: RemoteDownloadResultStatus::Success(outcome),
        },
        Err(error) => {
            if error == DOWNLOAD_RETRY_DEFERRED_ERROR {
                RemoteDownloadResult {
                    job_id: job.job_id,
                    remote_entry: job.remote_entry,
                    status: RemoteDownloadResultStatus::DeferredRetry,
                }
            } else if is_sync_cancelled_error(&error) {
                RemoteDownloadResult {
                    job_id: job.job_id,
                    remote_entry: job.remote_entry,
                    status: RemoteDownloadResultStatus::Cancelled,
                }
            } else {
                RemoteDownloadResult {
                    job_id: job.job_id,
                    remote_entry: job.remote_entry,
                    status: RemoteDownloadResultStatus::Failed(format!(
                        "Remote download failed item_id={} path={}: {}",
                        job.item_id, job.path, error
                    )),
                }
            }
        }
    }
}
