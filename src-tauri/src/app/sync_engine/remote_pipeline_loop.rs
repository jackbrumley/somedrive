struct RemotePipelineLoopOutcome {
    deferred_delta_link: Option<String>,
    remote_applied_paths: HashSet<String>,
}

// the pipeline loop owns the cycle's channels and shared counters explicitly; a pipeline context struct is tracked in docs/TODO.md.
#[allow(clippy::too_many_arguments)]
async fn run_remote_pipeline_loop(
    graph: &mut GraphContext,
    sync_root: &Path,
    sync_state: &mut PersistedSyncState,
    stats: &mut SyncCycleStats,
    lifecycle_state: &mut SyncLifecycleOperationalState,
    cycle_cancel_flag: &Arc<AtomicBool>,
    producer_alive: &Arc<AtomicBool>,
    active_download_workers: &Arc<AtomicUsize>,
    pending_download_count: &Arc<AtomicUsize>,
    page_rx: &mut mpsc::Receiver<Result<DeltaPageWorkItem, String>>,
    download_result_rx: &mut mpsc::Receiver<RemoteDownloadResult>,
    checkpoint_flush_step: usize,
) -> Result<RemotePipelineLoopOutcome, String> {
    const PHASE_PROGRESS_UPDATE_INTERVAL: Duration = Duration::from_millis(750);
    const PHASE_PROGRESS_ITEM_STEP: usize = 250;
    const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
    const DOWNLOAD_RESULT_BATCH_LIMIT: usize = 64;

    let scan_started_at = std::time::Instant::now();
    let mut last_phase_update_at = scan_started_at;
    let mut last_phase_update_items: usize = 0;
    let mut last_progress_at = scan_started_at;
    let stall_timeout = resolve_stall_timeout();
    let mut heartbeat_ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut watchdog_ticker = tokio::time::interval(Duration::from_secs(2));
    watchdog_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_page_progress_at: Option<std::time::Instant> = None;
    let mut last_download_result_at: Option<std::time::Instant> = None;
    let mut last_download_counter_signature: Option<(usize, usize, usize, usize, u64, u64, u64)> =
        None;
    let mut download_results_since_flush: usize = 0;
    let mut producer_done = false;
    let mut deferred_delta_link: Option<String> = None;
    let mut remote_applied_paths: HashSet<String> = HashSet::new();

    loop {
        ensure_not_cancelled(cycle_cancel_flag)?;
        let download_counters = sync_download_counters_from_db(graph)?;
        let current_counter_signature = (
            download_counters.remaining,
            download_counters.in_progress,
            download_counters.retry_waiting,
            download_counters.completed,
            download_counters.completed_bytes,
            download_counters.in_flight_bytes_done,
            download_counters.remaining_bytes,
        );
        if last_download_counter_signature != Some(current_counter_signature) {
            let now = std::time::Instant::now();
            last_progress_at = now;
            last_download_result_at = Some(now);
            last_download_counter_signature = Some(current_counter_signature);
        }
        let current_pending_downloads = pending_download_count.load(Ordering::Relaxed);
        if producer_done && current_pending_downloads == 0 && download_counters.remaining == 0 {
            break;
        }

        tokio::select! {
            biased;
            maybe_download_result = download_result_rx.recv() => {
                let download_result = match maybe_download_result {
                    Some(value) => value,
                    None => {
                        if producer_done && pending_download_count.load(Ordering::Relaxed) == 0 {
                            break;
                        }
                        return Err(format!(
                            "Download worker channel closed unexpectedly producer_alive={} active_workers={} pending_downloads={} remaining_download_jobs={} retry_waiting={}",
                            producer_alive.load(Ordering::Relaxed),
                            active_download_workers.load(Ordering::Relaxed),
                            pending_download_count.load(Ordering::Relaxed),
                            download_counters.remaining,
                            download_counters.retry_waiting
                        ));
                    },
                };
                last_progress_at = std::time::Instant::now();
                last_download_result_at = Some(last_progress_at);
                // Apply every result already waiting, then read the counters
                // once: per-result counter queries over the whole jobs table
                // let finished downloads pile up faster than they are saved.
                let mut result_batch = vec![download_result];
                while result_batch.len() < DOWNLOAD_RESULT_BATCH_LIMIT {
                    match download_result_rx.try_recv() {
                        Ok(next_result) => result_batch.push(next_result),
                        Err(_) => break,
                    }
                }
                for batched_result in result_batch {
                    apply_remote_download_result(
                        graph,
                        sync_root,
                        sync_state,
                        stats,
                        &mut remote_applied_paths,
                        batched_result,
                    )?;
                    download_results_since_flush += 1;
                }

                if download_results_since_flush >= checkpoint_flush_step {
                    save_sync_state(&graph.profile_id, sync_state)?;
                    download_results_since_flush = 0;
                }

                if !producer_done {
                    continue;
                }
                let latest_counters = sync_download_counters_from_db(graph)?;
                if latest_counters.remaining > 0 {
                    runtime_set_phase(
                        &graph.sync_runtime,
                        &graph.profile_id,
                        "applying_remote",
                        &format!("Finishing remote downloads - {} items remaining", latest_counters.remaining),
                    );
                }
            }
            _ = heartbeat_ticker.tick() => {
                log::info!(
                    "{} [cycle:{}] SYNC_HEARTBEAT phase=scanning_remote pages={} items={} pending_downloads={} remaining_download_jobs={} retry_waiting={}",
                    graph.account_prefix,
                    graph.cycle_id,
                    stats.remote_pages,
                    stats.remote_items_received,
                    pending_download_count.load(Ordering::Relaxed),
                    download_counters.remaining,
                    download_counters.retry_waiting
                );
            }
            _ = watchdog_ticker.tick() => {
                if last_progress_at.elapsed() >= stall_timeout {
                    let last_page_progress = last_page_progress_at
                        .map(|instant| format!("{}s", instant.elapsed().as_secs()))
                        .unwrap_or_else(|| "none".to_string());
                    let last_download_progress = last_download_result_at
                        .map(|instant| format!("{}s", instant.elapsed().as_secs()))
                        .unwrap_or_else(|| "none".to_string());
                    log::warn!(
                        "{} [cycle:{}] SYNC_STALLED phase=scanning_remote no_progress_for={}s pages={} items={} pending_downloads={} remaining_download_jobs={} in_progress={} retry_waiting={} producer_alive={} active_workers={} last_page_progress={} last_download_result_progress={}",
                        graph.account_prefix,
                        graph.cycle_id,
                        stall_timeout.as_secs(),
                        stats.remote_pages,
                        stats.remote_items_received,
                        pending_download_count.load(Ordering::Relaxed),
                        download_counters.remaining,
                        download_counters.in_progress,
                        download_counters.retry_waiting,
                        producer_alive.load(Ordering::Relaxed),
                        active_download_workers.load(Ordering::Relaxed),
                        last_page_progress,
                        last_download_progress
                    );
                    if download_counters.in_progress > 0 {
                        last_progress_at = std::time::Instant::now();
                        continue;
                    }
                    if producer_done && download_counters.retry_waiting > 0 {
                        last_progress_at = std::time::Instant::now();
                        continue;
                    }
                    return Err(format!(
                        "Remote sync stalled after {}s with no progress",
                        stall_timeout.as_secs()
                    ));
                }
            }
            maybe_page = page_rx.recv(), if !producer_done => {
                let page_payload = match maybe_page {
                    Some(Ok(value)) => value,
                    Some(Err(error)) => {
                        return Err(format!(
                            "Delta producer error={} producer_alive={} active_workers={} pending_downloads={} remaining_download_jobs={} retry_waiting={}",
                            error,
                            producer_alive.load(Ordering::Relaxed),
                            active_download_workers.load(Ordering::Relaxed),
                            pending_download_count.load(Ordering::Relaxed),
                            download_counters.remaining,
                            download_counters.retry_waiting
                        ));
                    },
                    None => {
                        producer_done = true;
                        continue;
                    }
                };
                last_progress_at = std::time::Instant::now();
                last_page_progress_at = Some(last_progress_at);

                stats.remote_pages += 1;
                stats.remote_items_received += page_payload.items.len();
                let has_next_link = page_payload.next_link.is_some();
                let should_update_phase = stats.remote_pages == 1
                    || last_phase_update_at.elapsed() >= PHASE_PROGRESS_UPDATE_INTERVAL
                    || stats
                        .remote_items_received
                        .saturating_sub(last_phase_update_items)
                        >= PHASE_PROGRESS_ITEM_STEP
                    || !has_next_link;
                if should_update_phase {
                    let elapsed_seconds = scan_started_at.elapsed().as_secs_f64();
                    runtime_set_phase(
                        &graph.sync_runtime,
                        &graph.profile_id,
                        "scanning_remote",
                        "Fetching remote files",
                    );
                    if elapsed_seconds > 0.0 {
                        log::info!(
                            "{} [cycle:{}] REMOTE_SCAN_PROGRESS pages={} items={} rate_items_per_sec={:.0}",
                            graph.account_prefix,
                            graph.cycle_id,
                            stats.remote_pages,
                            stats.remote_items_received,
                            stats.remote_items_received as f64 / elapsed_seconds
                        );
                    } else {
                        log::info!(
                            "{} [cycle:{}] REMOTE_SCAN_PROGRESS pages={} items={}",
                            graph.account_prefix,
                            graph.cycle_id,
                            stats.remote_pages,
                            stats.remote_items_received
                        );
                    }
                    last_phase_update_at = std::time::Instant::now();
                    last_phase_update_items = stats.remote_items_received;
                }
                log::info!(
                    "{} [cycle:{}] DELTA_PAGE_RECEIVED page={} items={}",
                    graph.account_prefix,
                    graph.cycle_id,
                    stats.remote_pages,
                    page_payload.items.len()
                );

                process_remote_page_items(
                    graph,
                    sync_root,
                    sync_state,
                    !lifecycle_state.two_way_ready,
                    cycle_cancel_flag,
                    page_payload.items,
                )
                .await?;
                let _ = recompute_sync_file_actions(&graph.profile_id, lifecycle_state.two_way_ready)?;
                let _ = materialize_planner_download_jobs(&graph.profile_id)?;
                let _ = sync_download_counters_from_db(graph)?;

                if let Some(next_link) = page_payload.next_link {
                    lifecycle_state.active_delta_next_link = Some(next_link);
                    persist_lifecycle_operational_state(&graph.profile_id, lifecycle_state)?;
                } else {
                    deferred_delta_link = page_payload.delta_link;
                    producer_done = true;
                }
            }
        }
    }

    Ok(RemotePipelineLoopOutcome {
        deferred_delta_link,
        remote_applied_paths,
    })
}
