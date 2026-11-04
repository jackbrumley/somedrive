fn graph_http_client_no_redirect() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(resolve_connect_timeout())
        .timeout(resolve_request_timeout())
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("Failed creating no-redirect Graph HTTP client: {error}"))
}

async fn graph_get_text(
    graph: &GraphContext,
    url: &str,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<String, String> {
    graph_get(graph, url, cancel_flag, false)
        .await?
        .ok_or_else(|| format!("Graph GET {url} returned no content"))
}

/// Like `graph_get_text`, but a 404 is an answer (`None`), not an error.
async fn graph_get_text_if_exists(
    graph: &GraphContext,
    url: &str,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<Option<String>, String> {
    graph_get(graph, url, cancel_flag, true).await
}

async fn graph_get(
    graph: &GraphContext,
    url: &str,
    cancel_flag: &Arc<AtomicBool>,
    not_found_is_none: bool,
) -> Result<Option<String>, String> {
    let client = graph_http_client()?;
    let mut refreshed = false;
    let mut attempt: u32 = 0;
    loop {
        ensure_not_cancelled(cancel_flag)?;
        graph.wait_for_throttle_window(cancel_flag).await?;
        attempt += 1;
        let access_token = graph.current_access_token().await;
        let response = tokio::select! {
            _ = wait_for_cancellation(Arc::clone(cancel_flag)) => {
                return Err(SYNC_CANCELLED_ERROR.to_string());
            }
            value = client
                .get(url)
                .bearer_auth(&access_token)
                .send() => {
                value.map_err(|error| format!("Graph GET failed: {error}"))
            }
        };
        let response = match response {
            Ok(value) => value,
            Err(error) => {
                if attempt < MAX_DOWNLOAD_RETRIES && is_transient_request_error(&error) {
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
                return Err(error);
            }
        };

        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            if status == StatusCode::TOO_MANY_REQUESTS {
                let _ = record_throttle_event(&graph.profile_id, DOWNLOAD_JOB_DIRECTION);
                graph.extend_throttle_window(
                    parse_retry_after_delay(response.headers())
                        .unwrap_or_else(|| exponential_backoff_delay(attempt)),
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

        let text = response
            .text()
            .await
            .map_err(|error| format!("Failed reading Graph response: {error}"));
        let text = match text {
            Ok(value) => value,
            Err(error) => {
                if attempt < MAX_DOWNLOAD_RETRIES && is_transient_request_error(&error) {
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
                return Err(error);
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

        if not_found_is_none && status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            let snippet: String = text.chars().take(400).collect();
            return Err(format!(
                "Graph GET {} failed with status {}: {}",
                url, status, snippet
            ));
        }
        return Ok(Some(text));
    }
}

fn is_transient_request_error(error: &str) -> bool {
    let normalized = error.to_lowercase();
    normalized.contains("timed out")
        || normalized.contains("timeout")
        || normalized.contains("connection reset")
        || normalized.contains("connection aborted")
        || normalized.contains("connection refused")
        || normalized.contains("tempor")
        || normalized.contains("dns")
        || normalized.contains("unreachable")
}

async fn graph_delete(graph: &GraphContext, url: &str) -> Result<(), String> {
    let client = graph_http_client()?;
    let mut refreshed = false;
    loop {
        let access_token = graph.current_access_token().await;
        let response = client
            .delete(url)
            .bearer_auth(&access_token)
            .send()
            .await
            .map_err(|error| format!("Graph DELETE failed: {error}"))?;
        let status = response.status();
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

        if status.is_success() || status.as_u16() == 404 {
            return Ok(());
        }

        let text = response.text().await.unwrap_or_default();
        let snippet: String = text.chars().take(400).collect();
        return Err(format!(
            "Graph DELETE {} failed with status {}: {}",
            url, status, snippet
        ));
    }
}

// the transfer needs the job, the item, the planned version and the target;
// a download request struct is tracked with the cycle context in docs/TODO.md.
#[allow(clippy::too_many_arguments)]
async fn download_remote_item_content(
    graph: &GraphContext,
    job_id: Option<i64>,
    item_id: &str,
    relative_path: &str,
    version: &CloudVersion,
    cloud_mtime: i64,
    local_path: &Path,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<RemoteDownloadOutcome, String> {
    ensure_not_cancelled(cancel_flag)?;
    log::info!(
        "{} [cycle:{}] DOWNLOAD_START item_id={} path={} local_path={}",
        graph.account_prefix,
        graph.cycle_id,
        item_id,
        relative_path,
        local_path.display()
    );
    let url = format!("{GRAPH_ROOT}/me/drive/items/{}/content", item_id);
    let graph_client = graph_http_client_no_redirect()?;
    let content_client = graph_http_client()?;
    let download_request_timeout = compute_download_request_timeout(Some(version.size));
    log::info!(
        "{} [cycle:{}] DOWNLOAD_TIMEOUT_POLICY path={} known_size_bytes={} timeout_seconds={}",
        graph.account_prefix,
        graph.cycle_id,
        relative_path,
        version.size,
        download_request_timeout.as_secs()
    );
    let mut token_refreshed = false;
    let mut attempt: u32 = 0;
    let transfer_id = runtime_start_transfer(
        &graph.sync_runtime,
        &graph.profile_id,
        "download",
        relative_path,
        None,
    );
    if let Some(active_job_id) = job_id {
        let _ = mark_download_job_running(&graph.profile_id, active_job_id);
    }
    loop {
        ensure_not_cancelled(cancel_flag)?;
        graph.wait_for_throttle_window(cancel_flag).await?;
        attempt += 1;
        if let Some(active_job_id) = job_id {
            let _ = mark_download_job_running(&graph.profile_id, active_job_id);
        }
        let access_token = graph.current_access_token().await;
        let response = tokio::select! {
            _ = wait_for_cancellation(Arc::clone(cancel_flag)) => {
                runtime_finish_transfer_cancelled(&graph.sync_runtime, &graph.profile_id, &transfer_id);
                return Err(SYNC_CANCELLED_ERROR.to_string());
            }
            value = graph_client
                .get(&url)
                .bearer_auth(&access_token)
                .timeout(download_request_timeout)
                .send() => {
                value.map_err(|error| format!("Download request failed: {error}"))
            }
        };
        let response = match response {
            Ok(value) => value,
            Err(error) => {
                if handle_download_retry(
                    graph,
                    job_id,
                    cancel_flag,
                    &transfer_id,
                    attempt,
                    relative_path,
                    &error,
                    None,
                    &error,
                )
                .await?
                {
                    continue;
                }
                return Err(error);
            }
        };
        let mut response = response;
        let mut status = response.status();
        if status.as_u16() == 401 && !token_refreshed {
            log::warn!(
                "{} [cycle:{}] GRAPH_DOWNLOAD_401_REFRESH",
                graph.account_prefix,
                graph.cycle_id
            );
            let _ = graph.refresh_token_if_needed(&access_token).await?;
            token_refreshed = true;
            continue;
        }
        if status.is_redirection() {
            let redirect_url = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .map(ToString::to_string);
            let redirect_url = match redirect_url {
                Some(value) if !value.trim().is_empty() => value,
                _ => {
                    let reason = format!(
                        "Download redirect missing location header status={}",
                        status
                    );
                    if handle_download_retry(
                        graph,
                        job_id,
                        cancel_flag,
                        &transfer_id,
                        attempt,
                        relative_path,
                        &reason,
                        None,
                        &reason,
                    )
                    .await?
                    {
                        continue;
                    }
                    return Err(reason);
                }
            };
            let redirect_host = reqwest::Url::parse(&redirect_url)
                .ok()
                .and_then(|value| value.host_str().map(ToString::to_string))
                .unwrap_or_else(|| "(unknown)".to_string());
            log::info!(
                "{} [cycle:{}] DOWNLOAD_REDIRECT item_id={} path={} status={} host={}",
                graph.account_prefix,
                graph.cycle_id,
                item_id,
                relative_path,
                status,
                redirect_host
            );
            let redirected_response = tokio::select! {
                _ = wait_for_cancellation(Arc::clone(cancel_flag)) => {
                    runtime_finish_transfer_cancelled(&graph.sync_runtime, &graph.profile_id, &transfer_id);
                    return Err(SYNC_CANCELLED_ERROR.to_string());
                }
                value = content_client
                    .get(&redirect_url)
                    .timeout(download_request_timeout)
                    .send() => {
                    value.map_err(|error| format!("Download redirect request failed: {error}"))
                }
            };
            response = match redirected_response {
                Ok(value) => value,
                Err(error) => {
                    if handle_download_retry(
                        graph,
                        job_id,
                        cancel_flag,
                        &transfer_id,
                        attempt,
                        relative_path,
                        &error,
                        None,
                        &error,
                    )
                    .await?
                    {
                        continue;
                    }
                    return Err(error);
                }
            };
            status = response.status();
            log::info!(
                "{} [cycle:{}] DOWNLOAD_REDIRECT_RESULT item_id={} path={} status={} host={}",
                graph.account_prefix,
                graph.cycle_id,
                item_id,
                relative_path,
                status,
                redirect_host
            );
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            let _ = record_throttle_event(&graph.profile_id, DOWNLOAD_JOB_DIRECTION);
            let response_headers = response.headers().clone();
            let response_text = response.text().await.unwrap_or_default();
            let delay = parse_retry_after_delay(&response_headers)
                .or_else(|| parse_retry_after_seconds_from_json_body(&response_text))
                .unwrap_or_else(|| exponential_backoff_delay(attempt));
            graph.extend_throttle_window(delay)?;
            if let Some(active_job_id) = job_id {
                let retry_reason = format!("Download retry scheduled for HTTP status {status}");
                if let Err(error) = mark_download_job_retry_wait(
                    &graph.profile_id,
                    active_job_id,
                    &retry_reason,
                    delay,
                ) {
                    log::warn!(
                        "{} [cycle:{}] DOWNLOAD_RETRY_WAIT_PERSIST_FAILED path={} error={}",
                        graph.account_prefix,
                        graph.cycle_id,
                        relative_path,
                        error
                    );
                }
            }
            runtime_finish_transfer_cancelled(&graph.sync_runtime, &graph.profile_id, &transfer_id);
            log::warn!(
                "{} [cycle:{}] DOWNLOAD_RETRY_DEFERRED status={} path={} delay_ms={}",
                graph.account_prefix,
                graph.cycle_id,
                status,
                relative_path,
                delay.as_millis()
            );
            return Err(DOWNLOAD_RETRY_DEFERRED_ERROR.to_string());
        }
        if status.is_server_error()
            && attempt < MAX_DOWNLOAD_RETRIES {
                let delay = parse_retry_after_delay(response.headers())
                    .unwrap_or_else(|| exponential_backoff_delay(attempt));
                if let Some(active_job_id) = job_id {
                    let retry_reason = format!("Download retry scheduled for HTTP status {status}");
                    if let Err(error) = record_download_job_retrying_in_place(
                        &graph.profile_id,
                        active_job_id,
                        &retry_reason,
                    ) {
                        log::warn!(
                            "{} [cycle:{}] DOWNLOAD_RETRY_PERSIST_FAILED path={} error={}",
                            graph.account_prefix,
                            graph.cycle_id,
                            relative_path,
                            error
                        );
                    }
                }
                log::warn!(
                    "{} [cycle:{}] DOWNLOAD_RETRY_HTTP attempt={} status={} path={} delay_ms={}",
                    graph.account_prefix,
                    graph.cycle_id,
                    attempt,
                    status,
                    relative_path,
                    delay.as_millis()
                );
                sleep_with_cancellation(cancel_flag, delay).await?;
                continue;
            }
        if !status.is_success() {
            if status == StatusCode::NOT_FOUND || status == StatusCode::GONE {
                if let Some(active_transfer_id) = &transfer_id {
                    runtime_finish_transfer_success(
                        &graph.sync_runtime,
                        &graph.profile_id,
                        active_transfer_id,
                    );
                }
                log::warn!(
                    "{} [cycle:{}] DOWNLOAD_NOT_FOUND item_id={} path={} status={}",
                    graph.account_prefix,
                    graph.cycle_id,
                    item_id,
                    relative_path,
                    status
                );
                return Ok(RemoteDownloadOutcome::NotFoundOnCloud);
            }
            let response_headers = response.headers().clone();
            let text = response.text().await.unwrap_or_default();
            let snippet: String = text.chars().take(400).collect();
            if status == StatusCode::FORBIDDEN {
                log_download_403_diagnostics(
                    graph,
                    item_id,
                    relative_path,
                    attempt,
                    job_id,
                    &response_headers,
                    &text,
                )
                .await;
            }
            if let Some(active_transfer_id) = &transfer_id {
                runtime_finish_transfer_error(
                    &graph.sync_runtime,
                    &graph.profile_id,
                    active_transfer_id,
                    &format!("Download failed with status {}", status),
                );
            }
            return Err(format!(
                "Download failed for item {} with status {}: {}",
                item_id, status, snippet
            ));
        }

        let total = response.content_length();
        if let Some(active_job_id) = job_id {
            let _ = update_download_job_progress(&graph.profile_id, active_job_id, 0, total);
        }
        if let Some(active_transfer_id) = &transfer_id {
            runtime_update_transfer_progress(
                &graph.sync_runtime,
                &graph.profile_id,
                active_transfer_id,
                0,
                total,
            );
        }

        if let Some(parent) = local_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Failed creating local parent '{}': {}",
                    parent.display(),
                    error
                )
            })?;
        }

        let temp_path = build_unique_download_temp_path(local_path, item_id, relative_path, &graph.cycle_id);
        let _ = std::fs::remove_file(&temp_path);
        let mut output_file = std::fs::File::create(&temp_path).map_err(|error| {
            format!(
                "Failed creating temporary download file '{}': {}",
                temp_path.display(),
                error
            )
        })?;

        let mut stream = response.bytes_stream();
        let mut downloaded_bytes: u64 = 0;
        let mut content_hasher = QuickXorHasher::new();
        let mut last_progress_persisted_at = std::time::Instant::now();
        let mut should_retry_download = false;
        while let Some(chunk_result) = stream.next().await {
            if cancel_flag.load(Ordering::Relaxed) {
                runtime_finish_transfer_cancelled(&graph.sync_runtime, &graph.profile_id, &transfer_id);
                let _ = std::fs::remove_file(&temp_path);
                return Err(SYNC_CANCELLED_ERROR.to_string());
            }
            let chunk = match chunk_result {
                Ok(value) => value,
                Err(error) => {
                    let reason = format!("Failed reading download stream: {error}");
                    if handle_download_retry(
                        graph,
                        job_id,
                        cancel_flag,
                        &transfer_id,
                        attempt,
                        relative_path,
                        &reason,
                        None,
                        &reason,
                    )
                    .await?
                    {
                        should_retry_download = true;
                        break;
                    }
                    let _ = std::fs::remove_file(&temp_path);
                    return Err(format!("Failed reading download bytes stream: {error}"));
                }
            };

            if let Err(error) = output_file.write_all(&chunk) {
                let reason = format!("Failed writing download chunk: {error}");
                if handle_download_retry(
                    graph,
                    job_id,
                    cancel_flag,
                    &transfer_id,
                    attempt,
                    relative_path,
                    &reason,
                    None,
                    &reason,
                )
                .await?
                {
                    should_retry_download = true;
                    break;
                }
                let _ = std::fs::remove_file(&temp_path);
                return Err(format!(
                    "Failed writing temporary file '{}': {}",
                    temp_path.display(),
                    error
                ));
            }

            downloaded_bytes += chunk.len() as u64;
            content_hasher.update(&chunk);
            // The database copy of progress is for restarts and the activity
            // projection, so it is written at most once per interval: a write
            // per chunk from every worker starves the result writer.
            if let Some(active_job_id) = job_id {
                if last_progress_persisted_at.elapsed() >= DOWNLOAD_PROGRESS_PERSIST_INTERVAL {
                    last_progress_persisted_at = std::time::Instant::now();
                    if let Err(error) = update_download_job_progress(
                        &graph.profile_id,
                        active_job_id,
                        downloaded_bytes,
                        total,
                    ) {
                        log::warn!(
                            "{} [cycle:{}] DOWNLOAD_PROGRESS_PERSIST_FAILED path={} error={}",
                            graph.account_prefix,
                            graph.cycle_id,
                            relative_path,
                            error
                        );
                    }
                }
            }
            if let Some(active_transfer_id) = &transfer_id {
                runtime_update_transfer_progress(
                    &graph.sync_runtime,
                    &graph.profile_id,
                    active_transfer_id,
                    downloaded_bytes,
                    total,
                );
            }
        }

        if should_retry_download {
            drop(output_file);
            let _ = std::fs::remove_file(&temp_path);
            continue;
        }

        let verification = output_file
            .flush()
            .and_then(|()| output_file.sync_all())
            .map_err(|error| format!("Failed flushing temporary file: {error}"))
            .and_then(|()| verify_downloaded_content(version, total, downloaded_bytes, &content_hasher));
        if let Err(reason) = verification {
            drop(output_file);
            if handle_download_retry(
                graph,
                job_id,
                cancel_flag,
                &transfer_id,
                attempt,
                relative_path,
                &reason,
                Some(&temp_path),
                &reason,
            )
            .await?
            {
                continue;
            }
            return Err(reason);
        }
        // OneDrive's time, so both sides agree on when the file changed.
        set_file_modified_seconds(&output_file, cloud_mtime);
        drop(output_file);

        if cancel_flag.load(Ordering::Relaxed) {
            runtime_finish_transfer_cancelled(&graph.sync_runtime, &graph.profile_id, &transfer_id);
            let _ = std::fs::remove_file(&temp_path);
            return Err(SYNC_CANCELLED_ERROR.to_string());
        }

        let placed_entry = match place_downloaded_file(graph, relative_path, &temp_path, local_path) {
            Ok(Some(entry)) => entry,
            Ok(None) => {
                runtime_finish_transfer_cancelled(&graph.sync_runtime, &graph.profile_id, &transfer_id);
                return Ok(RemoteDownloadOutcome::HeldAsConflict);
            }
            Err(reason) => {
                if handle_download_retry(
                    graph,
                    job_id,
                    cancel_flag,
                    &transfer_id,
                    attempt,
                    relative_path,
                    &reason,
                    Some(&temp_path),
                    &reason,
                )
                .await?
                {
                    continue;
                }
                return Err(reason);
            }
        };

        runtime_finish_transfer_download_success(
            &graph.sync_runtime,
            &graph.profile_id,
            &transfer_id,
            downloaded_bytes,
        );

        if attempt > 1 {
            log::info!(
                "{} [cycle:{}] DOWNLOAD_RETRY_RECOVERED attempts={} path={}",
                graph.account_prefix,
                graph.cycle_id,
                attempt,
                relative_path
            );
        }

        log::info!(
            "{} [cycle:{}] DOWNLOAD_OK item_id={} path={} bytes={}",
            graph.account_prefix,
            graph.cycle_id,
            item_id,
            relative_path,
            downloaded_bytes
        );
        return Ok(RemoteDownloadOutcome::Downloaded(placed_entry));
    }
}

async fn create_remote_folder(
    graph: &mut GraphContext,
    relative_path: &str,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<GraphDriveItem, String> {
    ensure_not_cancelled(cancel_flag)?;
    let (parent, name) = split_parent_and_name(relative_path)?;
    let endpoint = if parent.is_empty() {
        format!("{GRAPH_ROOT}/me/drive/root/children")
    } else {
        format!(
            "{GRAPH_ROOT}/me/drive/root:/{}:/children",
            encode_graph_path(parent)
        )
    };
    let payload = serde_json::json!({
        "name": name,
        "folder": {},
        "@microsoft.graph.conflictBehavior": "fail"
    });
    log::info!(
        "{} [cycle:{}] REMOTE_DIR_CREATE_REQUEST path={} parent={}",
        graph.account_prefix,
        graph.cycle_id,
        relative_path,
        parent
    );

    let client = graph_http_client()?;
    let mut refreshed = false;
    loop {
        ensure_not_cancelled(cancel_flag)?;
        graph.wait_for_throttle_window(cancel_flag).await?;
        let access_token = graph.current_access_token().await;
        let response = tokio::select! {
            _ = wait_for_cancellation(Arc::clone(cancel_flag)) => {
                return Err(SYNC_CANCELLED_ERROR.to_string());
            }
            value = client
                .post(&endpoint)
                .bearer_auth(&access_token)
                .json(&payload)
                .send() => {
                value.map_err(|error| {
                    format!(
                        "Failed creating remote folder '{}': {}",
                        relative_path, error
                    )
                })?
            }
        };

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| format!("Failed reading create folder response: {error}"))?;

        if status.as_u16() == 401 && !refreshed {
            log::warn!(
                "{} [cycle:{}] GRAPH_DIR_CREATE_401_REFRESH",
                graph.account_prefix,
                graph.cycle_id
            );
            let _ = graph.refresh_token_if_needed(&access_token).await?;
            refreshed = true;
            continue;
        }

        if status == StatusCode::CONFLICT {
            // Something already exists at this path: use it if it is a
            // folder, never replace it.
            let existing = fetch_remote_item_by_path(graph, relative_path, cancel_flag)
                .await?
                .filter(|item| item.folder.is_some())
                .ok_or_else(|| {
                    format!("A file in OneDrive is in the way of folder '{relative_path}'")
                })?;
            log::info!(
                "{} [cycle:{}] REMOTE_DIR_EXISTS path={} remote_id={}",
                graph.account_prefix,
                graph.cycle_id,
                relative_path,
                existing.id
            );
            return Ok(existing);
        }
        if !status.is_success() {
            let snippet: String = text.chars().take(400).collect();
            return Err(format!(
                "Create folder failed for '{}' with status {}: {}",
                relative_path, status, snippet
            ));
        }
        let parsed = serde_json::from_str::<GraphDriveItem>(&text)
            .map_err(|error| format!("Failed decoding create-folder response JSON: {error}"))?;
        log::info!(
            "{} [cycle:{}] REMOTE_DIR_CREATE_OK path={} remote_id={}",
            graph.account_prefix,
            graph.cycle_id,
            relative_path,
            parsed.id
        );
        return Ok(parsed);
    }
}

async fn delete_remote_item(
    graph: &mut GraphContext,
    item_id: &str,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(), String> {
    ensure_not_cancelled(cancel_flag)?;
    let url = format!("{GRAPH_ROOT}/me/drive/items/{}", item_id);
    log::info!(
        "{} [cycle:{}] REMOTE_DELETE_REQUEST item_id={}",
        graph.account_prefix,
        graph.cycle_id,
        item_id
    );
    graph_delete(graph, &url).await?;
    log::info!(
        "{} [cycle:{}] REMOTE_DELETE_RESULT item_id={} status=ok",
        graph.account_prefix,
        graph.cycle_id,
        item_id
    );
    Ok(())
}
