use super::*;
use futures_util::stream::{FuturesUnordered, StreamExt};

// Runs uploads and deletes many at a time under the same adaptive limit as
// downloads (`AdaptiveDownloadLimit`): as many at once as the limit allows,
// started no faster than its start rate, with big files kept to the
// large-file share of the slots. Microsoft's limit is per account, so one
// limit learned from every kind of request backs them all off together.

/// How often the pool looks again for start permits when it is waiting.
const POOL_POLL: Duration = Duration::from_millis(50);

/// Runs `work` for every item and hands each result to `finish`, one at a
/// time on the calling task, in the order they complete.
///
/// On cancellation (pause, shutdown) or the first error from `finish`, no
/// new work starts; work already running is allowed to finish and its
/// results still go to `finish` (a finished upload is still recorded), and
/// then the first error is returned. Work is never cut off mid-request.
pub(crate) async fn run_in_transfer_pool<T, R, W, Fut>(
    graph: &GraphContext,
    items: Vec<T>,
    is_large: impl Fn(&T) -> bool,
    cancel_flag: &Arc<AtomicBool>,
    work: W,
    mut finish: impl FnMut(R) -> SyncResult<()>,
) -> SyncResult<()>
where
    T: Send + 'static,
    R: Send + 'static,
    W: Fn(T) -> Fut,
    Fut: std::future::Future<Output = R> + Send + 'static,
{
    let mut queue: std::collections::VecDeque<T> = items.into();
    let mut running = FuturesUnordered::new();
    let mut running_large = 0usize;
    let mut first_error: Option<SyncError> = None;

    loop {
        if first_error.is_none() && cancel_flag.load(Ordering::Relaxed) {
            first_error = Some(SyncError::Cancelled);
        }
        if first_error.is_none() {
            let limit = &graph.download_limit;
            let free = limit
                .current()
                .saturating_sub(running.len())
                .min(queue.len());
            let granted = if free > 0 {
                limit.take_start_permits(free)
            } else {
                0
            };
            let mut started = 0;
            while started < granted {
                // A big file waits while the large-file share is in use;
                // small ones behind it still start.
                let large_full = running_large >= limit.large_download_slots();
                let Some(position) = queue
                    .iter()
                    .position(|item| !(large_full && is_large(item)))
                else {
                    break;
                };
                let Some(item) = queue.remove(position) else {
                    break;
                };
                let large = is_large(&item);
                running_large += usize::from(large);
                let task = work(item);
                running.push(tauri::async_runtime::spawn(
                    async move { (large, task.await) },
                ));
                started += 1;
            }
            limit.return_start_permits(granted - started);
        }

        if running.is_empty() {
            if queue.is_empty() || first_error.is_some() {
                break;
            }
            tokio::time::sleep(POOL_POLL).await;
            continue;
        }

        tokio::select! {
            joined = running.next() => {
                let Some(joined) = joined else { continue };
                match joined {
                    Ok((large, result)) => {
                        running_large -= usize::from(large);
                        if let Err(error) = finish(result) {
                            first_error.get_or_insert(error);
                        }
                    }
                    Err(error) => {
                        first_error.get_or_insert(SyncError::from(format!(
                            "A transfer task stopped unexpectedly: {error}"
                        )));
                    }
                }
            }
            _ = tokio::time::sleep(POOL_POLL), if first_error.is_none() && !queue.is_empty() => {}
        }
    }

    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(test)]
mod transfer_pool_tests {
    use super::*;

    #[test]
    fn every_item_runs_once_and_results_come_back() {
        tauri::async_runtime::block_on(async {
            let graph = test_graph_context("transfer-pool");
            let cancel_flag = Arc::new(AtomicBool::new(false));
            let mut finished = Vec::new();
            run_in_transfer_pool(
                &graph,
                (0..40).collect::<Vec<u32>>(),
                |item| *item % 10 == 0,
                &cancel_flag,
                |item| async move { item * 2 },
                |result| {
                    finished.push(result);
                    Ok(())
                },
            )
            .await
            .expect("pool");
            finished.sort_unstable();
            assert_eq!(finished, (0..40).map(|item| item * 2).collect::<Vec<u32>>());
        });
    }

    #[test]
    fn an_error_stops_new_work_but_running_work_still_finishes() {
        tauri::async_runtime::block_on(async {
            let graph = test_graph_context("transfer-pool-error");
            let cancel_flag = Arc::new(AtomicBool::new(false));
            let mut finished = 0usize;
            let result = run_in_transfer_pool(
                &graph,
                (0..1000).collect::<Vec<u32>>(),
                |_| false,
                &cancel_flag,
                |item| async move { item },
                |item| {
                    finished += 1;
                    if item == 0 {
                        Err(SyncError::from("stop".to_string()))
                    } else {
                        Ok(())
                    }
                },
            )
            .await;
            assert!(result.is_err());
            assert!(finished < 1000, "nothing new started after the error");
        });
    }

    #[test]
    fn cancelled_before_starting_runs_nothing() {
        tauri::async_runtime::block_on(async {
            let graph = test_graph_context("transfer-pool-cancel");
            let cancel_flag = Arc::new(AtomicBool::new(true));
            let result = run_in_transfer_pool(
                &graph,
                vec![1_u32, 2, 3],
                |_| false,
                &cancel_flag,
                |item| async move { item },
                |_| panic!("nothing should run"),
            )
            .await;
            assert!(matches!(result, Err(SyncError::Cancelled)));
        });
    }
}
