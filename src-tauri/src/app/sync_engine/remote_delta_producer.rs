fn spawn_delta_page_producer(
    graph: &GraphContext,
    start_url: String,
    cancel_flag: Arc<AtomicBool>,
    page_tx: mpsc::Sender<Result<DeltaPageWorkItem, String>>,
    producer_alive_flag: Arc<AtomicBool>,
) {
    let producer_graph = graph.clone();
    tauri::async_runtime::spawn(async move {
        producer_alive_flag.store(true, Ordering::Relaxed);
        log::info!(
            "{} [cycle:{}] DELTA_PRODUCER_STARTED",
            producer_graph.account_prefix,
            producer_graph.cycle_id
        );
        let mut current_url = start_url;
        loop {
            if let Err(error) = ensure_not_cancelled(&cancel_flag) {
                producer_alive_flag.store(false, Ordering::Relaxed);
                log::info!(
                    "{} [cycle:{}] DELTA_PRODUCER_STOP reason=cancelled",
                    producer_graph.account_prefix,
                    producer_graph.cycle_id
                );
                let _ = page_tx.send(Err(error)).await;
                return;
            }

            log::info!(
                "{} [cycle:{}] DELTA_PAGE_REQUEST url={}",
                producer_graph.account_prefix,
                producer_graph.cycle_id,
                current_url
            );
            let response_text = match graph_get_text(&producer_graph, &current_url, &cancel_flag).await {
                Ok(text) => text,
                Err(error) => {
                    producer_alive_flag.store(false, Ordering::Relaxed);
                    log::warn!(
                        "{} [cycle:{}] DELTA_PRODUCER_STOP reason=graph_get_error error={}",
                        producer_graph.account_prefix,
                        producer_graph.cycle_id,
                        error
                    );
                    let _ = page_tx.send(Err(error)).await;
                    return;
                }
            };

            let response: DeltaResponse = match serde_json::from_str(&response_text)
                .map_err(|error| format!("Failed to decode delta response: {error}"))
            {
                Ok(value) => value,
                Err(error) => {
                    producer_alive_flag.store(false, Ordering::Relaxed);
                    log::warn!(
                        "{} [cycle:{}] DELTA_PRODUCER_STOP reason=decode_error error={}",
                        producer_graph.account_prefix,
                        producer_graph.cycle_id,
                        error
                    );
                    let _ = page_tx.send(Err(error)).await;
                    return;
                }
            };

            let next_link = response.next_link.clone();
            let payload = DeltaPageWorkItem {
                items: response.value,
                next_link: response.next_link,
                delta_link: response.delta_link,
            };

            if page_tx.send(Ok(payload)).await.is_err() {
                producer_alive_flag.store(false, Ordering::Relaxed);
                log::info!(
                    "{} [cycle:{}] DELTA_PRODUCER_STOP reason=page_channel_closed",
                    producer_graph.account_prefix,
                    producer_graph.cycle_id
                );
                return;
            }

            if let Some(next) = next_link {
                current_url = next;
                continue;
            }

            producer_alive_flag.store(false, Ordering::Relaxed);
            log::info!(
                "{} [cycle:{}] DELTA_PRODUCER_STOP reason=scan_complete",
                producer_graph.account_prefix,
                producer_graph.cycle_id
            );
            return;
        }
    });
}
