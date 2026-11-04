use super::*;

// Downloads one file's content into a temporary file, with retries; the
// finishing steps are in download_finalize.rs.

#[allow(clippy::too_many_arguments)]
pub(crate) async fn download_remote_item_content(
    graph: &GraphContext,
    job_id: Option<i64>,
    item_id: &str,
    relative_path: &str,
    version: &CloudVersion,
    cloud_mtime: i64,
    direct_url: Option<&str>,
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
    let graph_client = graph.http_no_redirect.clone();
    let content_client = graph.content_http.clone();
    let mut token_refreshed = false;
    let mut attempt: u32 = 0;
    let transfer_id = runtime_start_transfer(
        &graph.sync_runtime,
        &graph.profile_id,
        "download",
        relative_path,
        None,
    );
    // Big files take Graph's redirect link and download in ranges.
    let segmented = version.size >= SEGMENTED_DOWNLOAD_MIN_BYTES;
    let mut direct_url = direct_url.filter(|_| !segmented).map(ToString::to_string);
    loop {
        ensure_not_cancelled(cancel_flag)?;
        let request_started_at = graph.wait_for_throttle_window(cancel_flag).await?;
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
            value = match direct_url.as_deref() {
                // The change feed's pre-authenticated link: one request
                // straight to the content server, no Graph call or redirect.
                Some(direct) => content_client.get(direct).send(),
                None => graph_client
                    .get(&url)
                    .bearer_auth(&access_token)
                    .send(),
            } => {
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
        if direct_url.is_some() && matches!(status.as_u16(), 401 | 403 | 404 | 410) {
            log::info!(
                "{} [cycle:{}] DOWNLOAD_DIRECT_URL_REFUSED path={} status={} action=use_graph",
                graph.account_prefix,
                graph.cycle_id,
                relative_path,
                status
            );
            direct_url = None;
            attempt -= 1;
            continue;
        }
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
            if segmented {
                let ranged = download_in_segments(
                    graph,
                    job_id,
                    &transfer_id,
                    relative_path,
                    item_id,
                    version,
                    &redirect_url,
                    local_path,
                    cancel_flag,
                )
                .await;
                let retry_reason = match ranged {
                    Err(error) if error == SYNC_CANCELLED_ERROR => {
                        runtime_finish_transfer_cancelled(
                            &graph.sync_runtime,
                            &graph.profile_id,
                            &transfer_id,
                        );
                        return Err(error);
                    }
                    Err(error) => Some(error),
                    Ok(SegmentedDownload::Complete { data_path, bytes }) => {
                        match finish_downloaded_file(
                            graph,
                            job_id,
                            &transfer_id,
                            attempt,
                            item_id,
                            relative_path,
                            &data_path,
                            local_path,
                            cloud_mtime,
                            bytes,
                            cancel_flag,
                        )
                        .await?
                        {
                            FinishStep::Done(outcome) => return Ok(outcome),
                            FinishStep::Retry => continue,
                        }
                    }
                    Ok(SegmentedDownload::Throttled { retry_after }) => {
                        return Err(defer_download_after_throttle(
                            graph,
                            job_id,
                            &transfer_id,
                            relative_path,
                            StatusCode::TOO_MANY_REQUESTS,
                            retry_after,
                            request_started_at,
                        )?);
                    }
                    Ok(SegmentedDownload::LinkExpired) => {
                        Some("The file server link expired; asking Graph for a new one".to_string())
                    }
                    Ok(SegmentedDownload::Failed(reason)) => Some(reason),
                    Ok(SegmentedDownload::RangesUnsupported) => {
                        log::warn!(
                            "{} [cycle:{}] DOWNLOAD_RANGES_UNSUPPORTED path={} action=single_stream",
                            graph.account_prefix,
                            graph.cycle_id,
                            relative_path
                        );
                        None
                    }
                };
                if let Some(reason) = retry_reason {
                    // Finished ranges stay on disk; the retry resumes them.
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
            }
            let redirected_response = tokio::select! {
                _ = wait_for_cancellation(Arc::clone(cancel_flag)) => {
                    runtime_finish_transfer_cancelled(&graph.sync_runtime, &graph.profile_id, &transfer_id);
                    return Err(SYNC_CANCELLED_ERROR.to_string());
                }
                value = content_client
                    .get(&redirect_url)
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
            let response_headers = response.headers().clone();
            let response_text = response.text().await.unwrap_or_default();
            let delay = parse_retry_after_delay(&response_headers)
                .or_else(|| parse_retry_after_seconds_from_json_body(&response_text))
                .unwrap_or_else(|| exponential_backoff_delay(attempt));
            return Err(defer_download_after_throttle(
                graph,
                job_id,
                &transfer_id,
                relative_path,
                status,
                delay,
                request_started_at,
            )?);
        }
        if status.is_server_error() && attempt < MAX_DOWNLOAD_RETRIES {
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

        let temp_path =
            build_unique_download_temp_path(local_path, item_id, relative_path, &graph.cycle_id);
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
                runtime_finish_transfer_cancelled(
                    &graph.sync_runtime,
                    &graph.profile_id,
                    &transfer_id,
                );
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
            .and_then(|()| {
                verify_downloaded_content(version, total, downloaded_bytes, &content_hasher)
            });
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
        drop(output_file);
        match finish_downloaded_file(
            graph,
            job_id,
            &transfer_id,
            attempt,
            item_id,
            relative_path,
            &temp_path,
            local_path,
            cloud_mtime,
            downloaded_bytes,
            cancel_flag,
        )
        .await?
        {
            FinishStep::Done(outcome) => return Ok(outcome),
            FinishStep::Retry => continue,
        }
    }
}
