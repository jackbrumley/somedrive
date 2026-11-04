async fn upload_file_by_path(
    graph: &mut GraphContext,
    sync_root: &Path,
    relative_path: &str,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<DriveItemResponse, String> {
    ensure_not_cancelled(cancel_flag)?;
    let local_path = sync_root.join(path_to_local(relative_path));
    let metadata = std::fs::metadata(&local_path).map_err(|error| {
        format!(
            "Failed reading local file metadata '{}': {}",
            local_path.display(),
            error
        )
    })?;
    let content_len = metadata.len();
    let modified_ts = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_secs() as i64)
        .unwrap_or(0);
    let upload_job_id = begin_upload_job(
        &graph.profile_id,
        relative_path,
        content_len,
        modified_ts,
        &graph.cycle_id,
    )?;
    sync_upload_counters_from_db(graph);
    let transfer_id = runtime_start_transfer(
        &graph.sync_runtime,
        &graph.profile_id,
        "upload",
        relative_path,
        Some(content_len),
    );
    log::info!(
        "{} [cycle:{}] UPLOAD_START path={} local_path={} bytes={}",
        graph.account_prefix,
        graph.cycle_id,
        relative_path,
        local_path.display(),
        content_len
    );

    let simple_upload_limit = resolve_simple_upload_max_bytes();
    if content_len <= simple_upload_limit {
        let content = std::fs::read(&local_path).map_err(|error| {
            format!(
                "Failed reading local file '{}': {}",
                local_path.display(),
                error
            )
        })?;
        let result = upload_small_file_simple(
            graph,
            upload_job_id,
            relative_path,
            content,
            content_len,
            transfer_id.as_deref(),
            cancel_flag,
        )
        .await;
        if let Err(error) = &result {
            if is_sync_cancelled_error(error) {
                let _ = mark_upload_job_retry_wait(
                    &graph.profile_id,
                    upload_job_id,
                    "Upload cancelled due to pause; retry scheduled on resume",
                    Duration::from_secs(1),
                );
            } else {
                match read_upload_job_attempt_count(&graph.profile_id, upload_job_id) {
                    Ok(attempt_count) if attempt_count >= MAX_DOWNLOAD_RETRIES => {
                        let _ = mark_upload_job_failed(&graph.profile_id, upload_job_id, error);
                    }
                    Ok(attempt_count) => {
                        let _ = mark_upload_job_retry_wait(
                            &graph.profile_id,
                            upload_job_id,
                            error,
                            resolve_upload_retry_delay(attempt_count),
                        );
                    }
                    Err(_) => {
                        let _ = mark_upload_job_failed(&graph.profile_id, upload_job_id, error);
                    }
                }
            }
        } else {
            let _ = mark_upload_job_done(&graph.profile_id, upload_job_id);
        }
        sync_upload_counters_from_db(graph);
        return result;
    }

    let result = upload_large_file_session(
        graph,
        upload_job_id,
        relative_path,
        &local_path,
        content_len,
        transfer_id.as_deref(),
        cancel_flag,
    )
    .await;
    if let Err(error) = &result {
        if is_sync_cancelled_error(error) {
            let _ = mark_upload_job_retry_wait(
                &graph.profile_id,
                upload_job_id,
                "Upload cancelled due to pause; retry scheduled on resume",
                Duration::from_secs(1),
            );
        } else {
            match read_upload_job_attempt_count(&graph.profile_id, upload_job_id) {
                Ok(attempt_count) if attempt_count >= MAX_DOWNLOAD_RETRIES => {
                    let _ = mark_upload_job_failed(&graph.profile_id, upload_job_id, error);
                }
                Ok(attempt_count) => {
                    let _ = mark_upload_job_retry_wait(
                        &graph.profile_id,
                        upload_job_id,
                        error,
                        resolve_upload_retry_delay(attempt_count),
                    );
                }
                Err(_) => {
                    let _ = mark_upload_job_failed(&graph.profile_id, upload_job_id, error);
                }
            }
        }
    } else {
        let _ = mark_upload_job_done(&graph.profile_id, upload_job_id);
    }
    sync_upload_counters_from_db(graph);
    result
}

fn sync_upload_counters_from_db(graph: &GraphContext) {
    match read_upload_job_counters(&graph.profile_id) {
        Ok(counters) => runtime_set_upload_counters(
            &graph.sync_runtime,
            &graph.profile_id,
            counters.planned_total,
            counters.completed,
            counters.failed_terminal,
            counters.in_progress,
        ),
        Err(error) => {
            log::warn!(
                "{} [cycle:{}] UPLOAD_COUNTERS_DB_SYNC_FAILED error={}",
                graph.account_prefix,
                graph.cycle_id,
                error
            );
        }
    }
}

fn resolve_upload_retry_delay(attempt_count: u32) -> Duration {
    let exponent = attempt_count.saturating_sub(1).min(10);
    let retry_seconds = 2_u64.saturating_pow(exponent).min(MAX_RETRY_DELAY_SECONDS);
    Duration::from_secs(retry_seconds.max(1))
}

async fn upload_small_file_simple(
    graph: &mut GraphContext,
    upload_job_id: i64,
    relative_path: &str,
    content: Vec<u8>,
    content_len: u64,
    transfer_id: Option<&str>,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<DriveItemResponse, String> {
    let encoded_path = encode_graph_path(relative_path);
    let url = format!("{GRAPH_ROOT}/me/drive/root:/{}:/content", encoded_path);
    let client = graph_http_client()?;
    let mut refreshed = false;
    loop {
        ensure_not_cancelled(cancel_flag)?;
        graph.wait_for_throttle_window(cancel_flag).await?;
        let access_token = graph.current_access_token().await;
        let response = tokio::select! {
            _ = wait_for_cancellation(Arc::clone(cancel_flag)) => {
                if let Some(active_transfer_id) = transfer_id {
                    runtime_finish_transfer_error(
                        &graph.sync_runtime,
                        &graph.profile_id,
                        active_transfer_id,
                        SYNC_CANCELLED_ERROR,
                    );
                }
                return Err(SYNC_CANCELLED_ERROR.to_string());
            }
            value = client
                .put(&url)
                .bearer_auth(&access_token)
                .body(content.clone())
                .send() => {
                value.map_err(|error| {
                    if let Some(active_transfer_id) = transfer_id {
                        runtime_finish_transfer_error(
                            &graph.sync_runtime,
                            &graph.profile_id,
                            active_transfer_id,
                            &format!("Upload request failed: {}", error),
                        );
                    }
                    format!("Failed uploading file '{}': {}", relative_path, error)
                })?
            }
        };

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| format!("Failed reading upload response: {error}"))?;

        ensure_not_cancelled(cancel_flag)?;

        if status.as_u16() == 401 && !refreshed {
            log::warn!(
                "{} [cycle:{}] GRAPH_UPLOAD_401_REFRESH",
                graph.account_prefix,
                graph.cycle_id
            );
            let _ = graph.refresh_token_if_needed(&access_token).await?;
            refreshed = true;
            continue;
        }

        if !status.is_success() {
            let snippet: String = text.chars().take(400).collect();
            if let Some(active_transfer_id) = transfer_id {
                runtime_finish_transfer_error(
                    &graph.sync_runtime,
                    &graph.profile_id,
                    active_transfer_id,
                    &format!("Upload failed with status {}", status),
                );
            }
            return Err(format!(
                "Upload failed for '{}' with status {}: {}",
                relative_path, status, snippet
            ));
        }
        let parsed = serde_json::from_str::<DriveItemResponse>(&text)
            .map_err(|error| format!("Failed decoding upload response JSON: {error}"))?;
        let _ = update_upload_job_progress(
            &graph.profile_id,
            upload_job_id,
            content_len,
            Some(content_len),
        );
        if let Some(active_transfer_id) = transfer_id {
            runtime_update_transfer_progress(
                &graph.sync_runtime,
                &graph.profile_id,
                active_transfer_id,
                content_len,
                Some(content_len),
            );
            runtime_finish_transfer_success(
                &graph.sync_runtime,
                &graph.profile_id,
                active_transfer_id,
            );
        }
        log::info!(
            "{} [cycle:{}] UPLOAD_OK path={} remote_id={} size={}",
            graph.account_prefix,
            graph.cycle_id,
            relative_path,
            parsed.id,
            parsed.size.unwrap_or(0)
        );
        return Ok(parsed);
    }
}

async fn upload_large_file_session(
    graph: &mut GraphContext,
    upload_job_id: i64,
    relative_path: &str,
    local_path: &Path,
    content_len: u64,
    transfer_id: Option<&str>,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<DriveItemResponse, String> {
    let upload_url = create_upload_session(graph, relative_path, cancel_flag).await?;
    let chunk_size = resolve_upload_chunk_bytes();
    let client = graph_http_client()?;
    let mut file = std::fs::File::open(local_path).map_err(|error| {
        format!(
            "Failed opening local upload file '{}': {}",
            local_path.display(),
            error
        )
    })?;
    let mut offset: u64 = 0;

    while offset < content_len {
        ensure_not_cancelled(cancel_flag)?;
        let remaining = (content_len - offset) as usize;
        let read_len = remaining.min(chunk_size);
        let mut chunk = vec![0_u8; read_len];
        file.read_exact(&mut chunk).map_err(|error| {
            format!(
                "Failed reading upload chunk '{}': {}",
                local_path.display(),
                error
            )
        })?;

        let chunk_start = offset;
        let chunk_end = offset + read_len as u64 - 1;
        let content_range = format!("bytes {}-{}/{}", chunk_start, chunk_end, content_len);

        let mut attempt: u32 = 0;
        loop {
            ensure_not_cancelled(cancel_flag)?;
            graph.wait_for_throttle_window(cancel_flag).await?;
            attempt += 1;
            let response = tokio::select! {
                _ = wait_for_cancellation(Arc::clone(cancel_flag)) => {
                    if let Some(active_transfer_id) = transfer_id {
                        runtime_finish_transfer_error(
                            &graph.sync_runtime,
                            &graph.profile_id,
                            active_transfer_id,
                            SYNC_CANCELLED_ERROR,
                        );
                    }
                    return Err(SYNC_CANCELLED_ERROR.to_string());
                }
                value = client
                    .put(&upload_url)
                    .header(reqwest::header::CONTENT_LENGTH, read_len)
                    .header(reqwest::header::CONTENT_RANGE, content_range.clone())
                    .body(chunk.clone())
                    .send() => {
                    value.map_err(|error| format!("Upload chunk request failed: {error}"))?
                }
            };

            let status = response.status();
            if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                if status == StatusCode::TOO_MANY_REQUESTS {
                    let _ = record_throttle_event(&graph.profile_id, UPLOAD_JOB_DIRECTION);
                    graph.extend_throttle_window(
                        parse_retry_after_delay(response.headers())
                            .unwrap_or_else(|| exponential_backoff_delay(attempt)),
                    )?;
                }
                if attempt < MAX_DOWNLOAD_RETRIES {
                    let delay = parse_retry_after_delay(response.headers())
                        .unwrap_or_else(|| exponential_backoff_delay(attempt));
                    log::warn!(
                        "{} [cycle:{}] UPLOAD_CHUNK_RETRY attempt={} status={} path={} range={} delay_ms={}",
                        graph.account_prefix,
                        graph.cycle_id,
                        attempt,
                        status,
                        relative_path,
                        content_range,
                        delay.as_millis()
                    );
                    sleep_with_cancellation(cancel_flag, delay).await?;
                    continue;
                }
            }

            let text = response.text().await.unwrap_or_default();
            if status == StatusCode::ACCEPTED {
                offset += read_len as u64;
                let _ = update_upload_job_progress(
                    &graph.profile_id,
                    upload_job_id,
                    offset,
                    Some(content_len),
                );
                if let Some(active_transfer_id) = transfer_id {
                    runtime_update_transfer_progress(
                        &graph.sync_runtime,
                        &graph.profile_id,
                        active_transfer_id,
                        offset,
                        Some(content_len),
                    );
                }
                break;
            }

            if status.is_success() {
                let parsed = serde_json::from_str::<DriveItemResponse>(&text).map_err(|error| {
                    format!("Failed decoding upload-session completion response JSON: {error}")
                })?;
                let _ = update_upload_job_progress(
                    &graph.profile_id,
                    upload_job_id,
                    content_len,
                    Some(content_len),
                );
                if let Some(active_transfer_id) = transfer_id {
                    runtime_update_transfer_progress(
                        &graph.sync_runtime,
                        &graph.profile_id,
                        active_transfer_id,
                        content_len,
                        Some(content_len),
                    );
                    runtime_finish_transfer_success(
                        &graph.sync_runtime,
                        &graph.profile_id,
                        active_transfer_id,
                    );
                }
                log::info!(
                    "{} [cycle:{}] UPLOAD_OK path={} remote_id={} size={}",
                    graph.account_prefix,
                    graph.cycle_id,
                    relative_path,
                    parsed.id,
                    parsed.size.unwrap_or(0)
                );
                return Ok(parsed);
            }

            if let Some(active_transfer_id) = transfer_id {
                runtime_finish_transfer_error(
                    &graph.sync_runtime,
                    &graph.profile_id,
                    active_transfer_id,
                    &format!("Upload failed with status {}", status),
                );
            }
            let snippet: String = text.chars().take(400).collect();
            return Err(format!(
                "Upload failed for '{}' with status {}: {}",
                relative_path, status, snippet
            ));
        }
    }

    Err(format!(
        "Upload session ended before completion for '{}'",
        relative_path
    ))
}

async fn create_upload_session(
    graph: &mut GraphContext,
    relative_path: &str,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<String, String> {
    let encoded_path = encode_graph_path(relative_path);
    let url = format!(
        "{GRAPH_ROOT}/me/drive/root:/{}:/createUploadSession",
        encoded_path
    );
    let payload = serde_json::json!({
        "item": {
            "@microsoft.graph.conflictBehavior": "replace"
        }
    });
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
                .post(&url)
                .bearer_auth(&access_token)
                .json(&payload)
                .send() => {
                value.map_err(|error| format!("Create upload session request failed: {error}"))?
            }
        };

        let status = response.status();
        if status.as_u16() == 401 && !refreshed {
            log::warn!(
                "{} [cycle:{}] GRAPH_UPLOAD_SESSION_401_REFRESH",
                graph.account_prefix,
                graph.cycle_id
            );
            let _ = graph.refresh_token_if_needed(&access_token).await?;
            refreshed = true;
            continue;
        }
        if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            let delay = parse_retry_after_delay(response.headers()).unwrap_or_else(|| Duration::from_secs(2));
            if status == StatusCode::TOO_MANY_REQUESTS {
                let _ = record_throttle_event(&graph.profile_id, UPLOAD_JOB_DIRECTION);
                graph.extend_throttle_window(delay)?;
            }
            log::warn!(
                "{} [cycle:{}] GRAPH_UPLOAD_SESSION_RETRY status={} path={} delay_ms={}",
                graph.account_prefix,
                graph.cycle_id,
                status,
                relative_path,
                delay.as_millis()
            );
            sleep_with_cancellation(cancel_flag, delay).await?;
            continue;
        }

        let text = response
            .text()
            .await
            .map_err(|error| format!("Failed reading create upload session response: {error}"))?;
        if !status.is_success() {
            let snippet: String = text.chars().take(400).collect();
            return Err(format!(
                "Create upload session failed for '{}' with status {}: {}",
                relative_path, status, snippet
            ));
        }

        let session = serde_json::from_str::<UploadSessionResponse>(&text)
            .map_err(|error| format!("Failed decoding create upload session response JSON: {error}"))?;
        return Ok(session.upload_url);
    }
}

#[cfg(test)]
mod upload_lane_tests {
    use super::*;

    #[test]
    fn resolve_upload_retry_delay_is_exponential_and_capped() {
        assert_eq!(resolve_upload_retry_delay(1), Duration::from_secs(1));
        assert_eq!(resolve_upload_retry_delay(2), Duration::from_secs(2));
        assert_eq!(resolve_upload_retry_delay(3), Duration::from_secs(4));
        assert_eq!(resolve_upload_retry_delay(4), Duration::from_secs(8));

        let capped = resolve_upload_retry_delay(32);
        assert_eq!(capped, Duration::from_secs(MAX_RETRY_DELAY_SECONDS));
    }
}
