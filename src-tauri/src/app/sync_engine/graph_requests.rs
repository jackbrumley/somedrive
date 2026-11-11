use super::*;

// Graph GET and DELETE requests with retries, throttling and token refresh.

/// The body of a successful GET, or the one status the caller asked to
/// handle itself (each caller handles a single status).
pub(crate) enum GraphGetOutcome {
    Body(String),
    HandledStatus,
}

/// Like `graph_get_text`, but a 404 is an answer (`None`), not an error.
pub(crate) async fn graph_get_text_if_exists(
    graph: &GraphContext,
    url: &str,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<Option<String>> {
    match graph_get(graph, url, cancel_flag, &[StatusCode::NOT_FOUND]).await? {
        GraphGetOutcome::Body(text) => Ok(Some(text)),
        GraphGetOutcome::HandledStatus => Ok(None),
    }
}

/// A delta page, or `None` when OneDrive says the delta link has expired
/// (410 Gone, `resyncRequired`) and the drive must be enumerated again.
pub(crate) async fn graph_get_delta_page(
    graph: &GraphContext,
    url: &str,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<Option<String>> {
    match graph_get(graph, url, cancel_flag, &[StatusCode::GONE]).await? {
        GraphGetOutcome::Body(text) => Ok(Some(text)),
        GraphGetOutcome::HandledStatus => Ok(None),
    }
}

pub(crate) async fn graph_get(
    graph: &GraphContext,
    url: &str,
    cancel_flag: &Arc<AtomicBool>,
    handled_statuses: &[StatusCode],
) -> SyncResult<GraphGetOutcome> {
    let client = graph.http.clone();
    let mut refreshed = false;
    let mut attempt: u32 = 0;
    loop {
        ensure_not_cancelled(cancel_flag)?;
        let request_started_at = graph.wait_for_throttle_window(cancel_flag).await?;
        attempt += 1;
        let access_token = graph.current_access_token().await;
        let response = tokio::select! {
            _ = wait_for_cancellation(Arc::clone(cancel_flag)) => {
                return Err(SyncError::Cancelled);
            }
            value = client
                .get(url)
                .bearer_auth(&access_token)
                .send() => value
        };
        let response = match response {
            Ok(value) => value,
            Err(error) => {
                if attempt < MAX_DOWNLOAD_RETRIES && is_transient_reqwest_error(&error) {
                    let delay = exponential_backoff_delay(attempt);
                    log::warn!(
                        "{} [cycle:{}] GRAPH_GET_RETRY attempt={} url={} reason={} delay_ms={}",
                        graph.account_prefix,
                        graph.cycle_id,
                        attempt,
                        url,
                        error,
                        delay.as_millis()
                    );
                    sleep_with_cancellation(cancel_flag, delay).await?;
                    continue;
                }
                return Err(SyncError::from_request(
                    &error,
                    format!("Graph GET failed: {error}"),
                ));
            }
        };

        let status = response.status();

        graph.note_rate_limit_headers(response.headers());
        if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            if status == StatusCode::TOO_MANY_REQUESTS {
                let _ = record_throttle_event(&graph.profile_id, DOWNLOAD_JOB_DIRECTION);
                graph.extend_throttle_window(
                    parse_retry_after_delay(response.headers())
                        .unwrap_or_else(|| exponential_backoff_delay(attempt)),
                    request_started_at,
                )?;
            }
            if attempt < MAX_DOWNLOAD_RETRIES {
                let delay = parse_retry_after_delay(response.headers())
                    .unwrap_or_else(|| exponential_backoff_delay(attempt));
                log::warn!(
                    "{} [cycle:{}] GRAPH_GET_RETRY_HTTP attempt={} status={} url={} delay_ms={}",
                    graph.account_prefix,
                    graph.cycle_id,
                    attempt,
                    status,
                    url,
                    delay.as_millis()
                );
                sleep_with_cancellation(cancel_flag, delay).await?;
                continue;
            }
        }

        let text = match response.text().await {
            Ok(value) => value,
            Err(error) => {
                if attempt < MAX_DOWNLOAD_RETRIES && is_transient_reqwest_error(&error) {
                    let delay = exponential_backoff_delay(attempt);
                    log::warn!(
                        "{} [cycle:{}] GRAPH_GET_RETRY_BODY attempt={} url={} reason={} delay_ms={}",
                        graph.account_prefix,
                        graph.cycle_id,
                        attempt,
                        url,
                        error,
                        delay.as_millis()
                    );
                    sleep_with_cancellation(cancel_flag, delay).await?;
                    continue;
                }
                return Err(SyncError::from_request(
                    &error,
                    format!("Failed reading Graph response: {error}"),
                ));
            }
        };

        if status.as_u16() == 401 && !refreshed {
            log::warn!(
                "{} [cycle:{}] GRAPH_GET_401_REFRESH",
                graph.account_prefix,
                graph.cycle_id
            );
            let _ = graph.refresh_token_if_needed(&access_token).await?;
            refreshed = true;
            continue;
        }

        if handled_statuses.contains(&status) {
            return Ok(GraphGetOutcome::HandledStatus);
        }
        if !status.is_success() {
            let snippet: String = text.chars().take(400).collect();
            return Err(SyncError::from_status(
                status,
                format!(
                    "Graph GET {} failed with status {}: {}",
                    url, status, snippet
                ),
            ));
        }
        return Ok(GraphGetOutcome::Body(text));
    }
}

/// A request that failed for a reason worth retrying (connection, timeout,
/// interrupted body), judged from the error itself.
pub(crate) fn is_transient_reqwest_error(error: &reqwest::Error) -> bool {
    error.is_connect() || error.is_timeout() || error.is_request() || error.is_body()
}

/// What a Graph DELETE did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GraphDeleteOutcome {
    /// Deleted.
    Deleted,
    /// Already gone: nothing to delete.
    NotFound,
    /// The item no longer matched the `If-Match` eTag, so nothing was deleted.
    ChangedSinceRead,
}

/// Deletes `url`. With `if_match`, OneDrive deletes only while the item's
/// eTag is still that value. Waits out the account's throttle window like
/// every other request, and retries throttling, server errors and dropped
/// connections, so many deletes can run at once.
pub(crate) async fn graph_delete(
    graph: &GraphContext,
    url: &str,
    if_match: Option<&str>,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<GraphDeleteOutcome> {
    let client = graph.http.clone();
    let mut refreshed = false;
    let mut attempt: u32 = 0;
    loop {
        ensure_not_cancelled(cancel_flag)?;
        let request_started_at = graph.wait_for_throttle_window(cancel_flag).await?;
        attempt += 1;
        let access_token = graph.current_access_token().await;
        let mut request = client.delete(url).bearer_auth(&access_token);
        if let Some(etag) = if_match {
            request = request.header(reqwest::header::IF_MATCH, etag);
        }
        let response = tokio::select! {
            _ = wait_for_cancellation(Arc::clone(cancel_flag)) => {
                return Err(SyncError::Cancelled);
            }
            value = request.send() => value
        };
        let response = match response {
            Ok(value) => value,
            Err(error) => {
                if attempt < MAX_DOWNLOAD_RETRIES && is_transient_reqwest_error(&error) {
                    let delay = exponential_backoff_delay(attempt);
                    log::warn!(
                        "{} [cycle:{}] GRAPH_DELETE_RETRY attempt={} url={} reason={} delay_ms={}",
                        graph.account_prefix,
                        graph.cycle_id,
                        attempt,
                        url,
                        error,
                        delay.as_millis()
                    );
                    sleep_with_cancellation(cancel_flag, delay).await?;
                    continue;
                }
                return Err(SyncError::from_request(
                    &error,
                    format!("Graph DELETE failed: {error}"),
                ));
            }
        };
        let status = response.status();
        graph.note_rate_limit_headers(response.headers());
        if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            let delay = parse_retry_after_delay(response.headers())
                .unwrap_or_else(|| exponential_backoff_delay(attempt));
            if status == StatusCode::TOO_MANY_REQUESTS {
                let _ = record_throttle_event(&graph.profile_id, DOWNLOAD_JOB_DIRECTION);
                graph.extend_throttle_window(delay, request_started_at)?;
            }
            if attempt < MAX_DOWNLOAD_RETRIES {
                log::warn!(
                    "{} [cycle:{}] GRAPH_DELETE_RETRY_HTTP attempt={} status={} url={} delay_ms={}",
                    graph.account_prefix,
                    graph.cycle_id,
                    attempt,
                    status,
                    url,
                    delay.as_millis()
                );
                sleep_with_cancellation(cancel_flag, delay).await?;
                continue;
            }
        }
        if status.as_u16() == 401 && !refreshed {
            log::warn!(
                "{} [cycle:{}] GRAPH_DELETE_401_REFRESH",
                graph.account_prefix,
                graph.cycle_id
            );
            let _ = graph.refresh_token_if_needed(&access_token).await?;
            refreshed = true;
            continue;
        }

        if status.is_success() {
            return Ok(GraphDeleteOutcome::Deleted);
        }
        if status == StatusCode::NOT_FOUND {
            return Ok(GraphDeleteOutcome::NotFound);
        }
        if status.as_u16() == 412 && if_match.is_some() {
            return Ok(GraphDeleteOutcome::ChangedSinceRead);
        }

        let text = response.text().await.unwrap_or_default();
        let snippet: String = text.chars().take(400).collect();
        return Err(SyncError::from_status(
            status,
            format!("Graph DELETE {url} failed with status {status}: {snippet}"),
        ));
    }
}
