use super::*;

/// Pages through the cloud changes since the last cycle (the whole drive on
/// the first sync), recording each page and downloading planned files while
/// pages are still arriving.
pub(crate) async fn fetch_and_apply_delta_changes(
    graph: &mut GraphContext,
    sync_root: &Path,
    stats: &mut SyncCycleStats,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<()> {
    let delta_page_queue_capacity = resolve_delta_page_queue_capacity();
    let download_queue_capacity = resolve_download_queue_capacity();
    let download_concurrency = graph.download_limit.maximum();

    runtime_set_phase(
        &graph.sync_runtime,
        &graph.profile_id,
        "scanning_remote",
        "Fetching remote files",
    );
    runtime_set_remote_scan_complete(&graph.sync_runtime, &graph.profile_id, false);
    runtime_set_remote_download_counters(&graph.sync_runtime, &graph.profile_id, 0, 0, 0, 0, 0);
    log::info!(
        "{} [cycle:{}] REMOTE_PIPELINE_START delta_queue_capacity={} download_queue_capacity={} download_limit_start={} download_limit_max={}",
        graph.account_prefix,
        graph.cycle_id,
        delta_page_queue_capacity,
        download_queue_capacity,
        graph.download_limit.current(),
        download_concurrency
    );

    let mut lifecycle_state = read_sync_lifecycle_operational_state(&graph.profile_id)?;
    let start_url = lifecycle_state
        .active_delta_next_link
        .clone()
        .or_else(|| lifecycle_state.delta_link.clone())
        .unwrap_or_else(|| format!("{GRAPH_ROOT}/me/drive/root/delta"));
    let cycle_cancel_flag = Arc::new(AtomicBool::new(false));
    let cycle_cancel_bridge_flag = Arc::clone(&cycle_cancel_flag);
    let global_cancel_flag = Arc::clone(cancel_flag);
    tauri::async_runtime::spawn(async move {
        while !cycle_cancel_bridge_flag.load(Ordering::Relaxed) {
            if global_cancel_flag.load(Ordering::Relaxed) {
                cycle_cancel_bridge_flag.store(true, Ordering::Relaxed);
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    let producer_alive = Arc::new(AtomicBool::new(false));
    let active_download_workers = Arc::new(AtomicUsize::new(0));

    let (page_tx, mut page_rx) =
        mpsc::channel::<Result<DeltaPageWorkItem, SyncError>>(delta_page_queue_capacity);
    let producer_alive_flag = Arc::clone(&producer_alive);
    spawn_delta_page_producer(
        graph,
        start_url,
        Arc::clone(&cycle_cancel_flag),
        page_tx,
        producer_alive_flag,
    );

    let (download_tx_raw, download_rx) =
        mpsc::channel::<RemoteDownloadJob>(download_queue_capacity);
    let mut download_tx = Some(download_tx_raw);
    let (download_result_tx, mut download_result_rx) =
        mpsc::channel::<RemoteDownloadResult>(download_queue_capacity);
    let download_rx = Arc::new(tokio::sync::Mutex::new(download_rx));
    let pending_download_count = Arc::new(AtomicUsize::new(0));

    spawn_remote_download_workers(
        graph,
        download_concurrency,
        &cycle_cancel_flag,
        Arc::clone(&download_rx),
        download_result_tx.clone(),
        Arc::clone(&active_download_workers),
        Arc::clone(&pending_download_count),
    );
    drop(download_result_tx);

    let dispatcher_stop = Arc::new(AtomicBool::new(false));
    let dispatcher_stop_flag = Arc::clone(&dispatcher_stop);
    let dispatcher_download_tx = download_tx
        .as_ref()
        .ok_or_else(|| "Remote download queue unavailable".to_string())?
        .clone();
    spawn_remote_download_dispatcher(
        graph,
        sync_root,
        Arc::clone(&pending_download_count),
        dispatcher_stop_flag,
        dispatcher_download_tx,
    );

    let loop_outcome = run_remote_pipeline_loop(
        graph,
        sync_root,
        stats,
        &cycle_cancel_flag,
        &producer_alive,
        &active_download_workers,
        &pending_download_count,
        &mut page_rx,
        &mut download_result_rx,
    )
    .await;

    cycle_cancel_flag.store(true, Ordering::Relaxed);
    dispatcher_stop.store(true, Ordering::Relaxed);
    let _ = download_tx.take();
    let loop_outcome = loop_outcome?;
    log::info!(
        "{} [cycle:{}] REMOTE_PIPELINE_DRAIN producer_alive={} active_workers={} pending_downloads={} remaining_download_jobs={}",
        graph.account_prefix,
        graph.cycle_id,
        producer_alive.load(Ordering::Relaxed),
        active_download_workers.load(Ordering::Relaxed),
        pending_download_count.load(Ordering::Relaxed),
        sync_download_counters_from_db(graph)?.remaining
    );
    if let Some(delta_link) = loop_outcome.deferred_delta_link {
        lifecycle_state.delta_link = Some(delta_link);
    }
    lifecycle_state.active_delta_next_link = None;
    persist_lifecycle_operational_state(&graph.profile_id, &lifecycle_state)?;
    confirm_items_missing_after_resync(graph, cancel_flag).await?;
    let _ = sync_download_counters_from_db(graph)?;
    runtime_set_remote_scan_complete(&graph.sync_runtime, &graph.profile_id, true);
    log::info!(
        "{} [cycle:{}] REMOTE_PIPELINE_COMPLETE discovered={} downloaded={} skipped_unresolved={} producer_alive={} active_workers={}",
        graph.account_prefix,
        graph.cycle_id,
        stats.remote_items_received,
        stats.downloaded_files,
        stats.remote_items_skipped_unresolved,
        producer_alive.load(Ordering::Relaxed),
        active_download_workers.load(Ordering::Relaxed)
    );

    Ok(())
}

/// After a fresh enumeration (expired delta link), items it did not return
/// are not taken as deleted: each is read directly, and only an item
/// OneDrive no longer has is recorded as deleted.
pub(crate) async fn confirm_items_missing_after_resync(
    graph: &GraphContext,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<()> {
    let Some(started_at) = read_delta_resync_started_at(&graph.profile_id)? else {
        return Ok(());
    };
    let unseen = list_cloud_items_unseen_since(&graph.profile_id, started_at)?;
    let mut confirmed_deleted = 0;
    for (path, item_id) in &unseen {
        ensure_not_cancelled(cancel_flag)?;
        if fetch_remote_item(graph, item_id, cancel_flag)
            .await?
            .is_none()
        {
            record_cloud_item_deleted(&graph.profile_id, item_id)?;
            confirmed_deleted += 1;
            log::info!(
                "{} [cycle:{}] DELTA_RESYNC_CONFIRMED_DELETED path={}",
                graph.account_prefix,
                graph.cycle_id,
                path
            );
        }
    }
    finish_delta_resync(&graph.profile_id)?;
    log::info!(
        "{} [cycle:{}] DELTA_RESYNC_COMPLETE unseen={} confirmed_deleted={}",
        graph.account_prefix,
        graph.cycle_id,
        unseen.len(),
        confirmed_deleted
    );
    Ok(())
}
