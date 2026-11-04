use super::*;

pub(crate) fn spawn_remote_download_dispatcher(
    graph: &GraphContext,
    sync_root: &Path,
    pending_download_count: Arc<AtomicUsize>,
    dispatcher_stop_flag: Arc<AtomicBool>,
    dispatcher_download_tx: mpsc::Sender<RemoteDownloadJob>,
) {
    let dispatcher_graph = graph.clone();
    let dispatcher_sync_root = sync_root.to_path_buf();
    tauri::async_runtime::spawn(async move {
        let mut dispatch_ticker = tokio::time::interval(Duration::from_millis(50));
        dispatch_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            dispatch_ticker.tick().await;
            if dispatcher_stop_flag.load(Ordering::Relaxed) {
                break;
            }

            let pending = pending_download_count.load(Ordering::Relaxed);
            let available_slots = dispatcher_graph
                .download_limit
                .current()
                .saturating_sub(pending);
            if available_slots == 0 {
                continue;
            }
            // Free slots are filled no faster than the start rate allows.
            let available_slots = dispatcher_graph
                .download_limit
                .take_start_permits(available_slots);
            if available_slots == 0 {
                continue;
            }

            // No queue count here: the claim itself finds nothing when the
            // queue is empty, and counting 15,000 jobs 20 times a second
            // cost a third of a core.
            match dispatch_claimed_download_jobs(
                &dispatcher_graph,
                &dispatcher_sync_root,
                &dispatcher_download_tx,
                available_slots,
                pending,
            )
            .await
            {
                Ok(dispatched) => {
                    if dispatched > 0 {
                        pending_download_count.fetch_add(dispatched, Ordering::Relaxed);
                    }
                    dispatcher_graph
                        .download_limit
                        .return_start_permits(available_slots.saturating_sub(dispatched));
                }
                Err(error) => {
                    if error.contains("queue closed unexpectedly") {
                        log::info!(
                            "{} [cycle:{}] DOWNLOAD_DISPATCHER_STOP reason=queue_closed",
                            dispatcher_graph.account_prefix,
                            dispatcher_graph.cycle_id
                        );
                        break;
                    }
                    log::warn!(
                        "{} [cycle:{}] DOWNLOAD_DISPATCHER_FAILED error={}",
                        dispatcher_graph.account_prefix,
                        dispatcher_graph.cycle_id,
                        error
                    );
                }
            }
        }
    });
}
