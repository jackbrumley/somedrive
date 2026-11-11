use super::*;
use std::io::{Seek, SeekFrom};

// Big uploads go through an upload session: the file is sent in pieces of
// `resolve_upload_chunk_bytes`, which OneDrive requires in order (so one
// file's pieces cannot go in parallel; several files can, through the
// transfer pool). The session's address is saved, so an upload cut off by
// a pause, a restart or a dropped connection continues where it stopped
// instead of sending the whole file again.

/// A saved session is not reused when it expires within this long.
const SESSION_EXPIRY_MARGIN_SECONDS: i64 = 120;

/// An upload session saved for resuming, valid only for the same file.
struct SavedUploadSession {
    upload_url: String,
    local_size: u64,
    local_mtime_ns: i64,
    expires_at: i64,
}

fn read_saved_upload_session(
    profile_id: &str,
    path: &str,
) -> Result<Option<SavedUploadSession>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .query_row(
            "SELECT upload_url, local_size, local_mtime_ns, expires_at
             FROM sync_upload_sessions WHERE profile_id = ?1 AND path = ?2",
            params![profile_id, path],
            |row| {
                Ok(SavedUploadSession {
                    upload_url: row.get(0)?,
                    local_size: row.get::<_, i64>(1)?.max(0) as u64,
                    local_mtime_ns: row.get(2)?,
                    expires_at: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("Failed reading the saved upload session for '{path}': {error}"))
}

fn save_upload_session(
    profile_id: &str,
    path: &str,
    session: &SavedUploadSession,
) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "INSERT INTO sync_upload_sessions
                 (profile_id, path, upload_url, local_size, local_mtime_ns, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(profile_id, path) DO UPDATE SET
                 upload_url = excluded.upload_url,
                 local_size = excluded.local_size,
                 local_mtime_ns = excluded.local_mtime_ns,
                 expires_at = excluded.expires_at",
            params![
                profile_id,
                path,
                session.upload_url,
                session.local_size as i64,
                session.local_mtime_ns,
                session.expires_at
            ],
        )
        .map_err(|error| format!("Failed saving the upload session for '{path}': {error}"))?;
    Ok(())
}

pub(crate) fn forget_upload_session(profile_id: &str, path: &str) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "DELETE FROM sync_upload_sessions WHERE profile_id = ?1 AND path = ?2",
            params![profile_id, path],
        )
        .map_err(|error| format!("Failed forgetting the upload session for '{path}': {error}"))?;
    Ok(())
}

/// The session for this exact file (same size and modified time) that has
/// not expired, if one was saved.
fn reusable_upload_session(
    profile_id: &str,
    path: &str,
    entry: &LocalEntry,
) -> Result<Option<SavedUploadSession>, String> {
    let Some(saved) = read_saved_upload_session(profile_id, path)? else {
        return Ok(None);
    };
    let still_valid = saved.local_size == entry.size
        && saved.local_mtime_ns == entry.mtime_ns
        && saved.expires_at > current_unix_seconds() + SESSION_EXPIRY_MARGIN_SECONDS;
    if still_valid {
        return Ok(Some(saved));
    }
    forget_upload_session(profile_id, path)?;
    Ok(None)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadSessionStatus {
    #[serde(default)]
    next_expected_ranges: Vec<String>,
}

/// Where OneDrive wants the upload to continue: the start of its first
/// expected range ("12345-" or "12345-67890"). `None` when the session is
/// gone or says nothing useful, so a new session is started.
async fn session_resume_offset(
    graph: &GraphContext,
    upload_url: &str,
    content_len: u64,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<Option<u64>, String> {
    // The session address carries its own authorisation: no token is sent.
    let response = tokio::select! {
        _ = wait_for_cancellation(Arc::clone(cancel_flag)) => {
            return Err(SYNC_CANCELLED_ERROR.to_string());
        }
        value = graph.content_http.get(upload_url).send() => value
    };
    let Ok(response) = response else {
        return Ok(None);
    };
    if !response.status().is_success() {
        return Ok(None);
    }
    let Ok(status) = response.json::<UploadSessionStatus>().await else {
        return Ok(None);
    };
    Ok(status
        .next_expected_ranges
        .first()
        .and_then(|range| range.split('-').next())
        .and_then(|start| start.trim().parse::<u64>().ok())
        .filter(|start| *start < content_len))
}

/// Seconds since the epoch for OneDrive's `expirationDateTime`; an unknown
/// expiry counts as one day from now.
fn session_expiry(expiration: Option<&str>) -> i64 {
    expiration
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp())
        .unwrap_or_else(|| current_unix_seconds() + 24 * 60 * 60)
}

// see upload_small_file_simple.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn upload_large_file_session(
    graph: &mut GraphContext,
    upload_job_id: i64,
    relative_path: &str,
    target: &UploadTarget,
    local_path: &Path,
    content_len: u64,
    transfer_id: Option<&str>,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<GraphDriveItem, String> {
    let entry = read_local_entry(local_path)?
        .ok_or_else(|| format!("'{}' is no longer on this device", local_path.display()))?;
    let mut resumed = None;
    if let Some(saved) = reusable_upload_session(&graph.profile_id, relative_path, &entry)? {
        match session_resume_offset(graph, &saved.upload_url, content_len, cancel_flag).await? {
            Some(offset) => resumed = Some((saved.upload_url, offset)),
            None => forget_upload_session(&graph.profile_id, relative_path)?,
        }
    }
    let (upload_url, mut offset) = match resumed {
        Some((upload_url, offset)) => {
            log::info!(
                "{} [cycle:{}] UPLOAD_SESSION_RESUMED path={} offset={} bytes={}",
                graph.account_prefix,
                graph.cycle_id,
                relative_path,
                offset,
                content_len
            );
            (upload_url, offset)
        }
        None => {
            let session = create_upload_session(graph, relative_path, target, cancel_flag).await?;
            save_upload_session(
                &graph.profile_id,
                relative_path,
                &SavedUploadSession {
                    upload_url: session.upload_url.clone(),
                    local_size: entry.size,
                    local_mtime_ns: entry.mtime_ns,
                    expires_at: session_expiry(session.expiration_date_time.as_deref()),
                },
            )?;
            (session.upload_url, 0)
        }
    };

    let chunk_size = resolve_upload_chunk_bytes();
    let client = graph.content_http.clone();
    let mut file = std::fs::File::open(local_path).map_err(|error| {
        format!(
            "Failed opening local upload file '{}': {}",
            local_path.display(),
            error
        )
    })?;
    file.seek(SeekFrom::Start(offset)).map_err(|error| {
        format!(
            "Failed seeking in local upload file '{}': {}",
            local_path.display(),
            error
        )
    })?;
    report_upload_progress(graph, upload_job_id, transfer_id, offset, content_len);

    while offset < content_len {
        ensure_not_cancelled(cancel_flag)?;
        let remaining = (content_len - offset) as usize;
        let read_len = remaining.min(chunk_size);
        let mut buffer = vec![0_u8; read_len];
        file.read_exact(&mut buffer).map_err(|error| {
            format!(
                "Failed reading upload chunk '{}': {}",
                local_path.display(),
                error
            )
        })?;
        // Shared, so a retry resends the same piece without copying it.
        let chunk = bytes::Bytes::from(buffer);

        let chunk_end = offset + read_len as u64 - 1;
        let content_range = format!("bytes {}-{}/{}", offset, chunk_end, content_len);

        let mut attempt: u32 = 0;
        loop {
            ensure_not_cancelled(cancel_flag)?;
            let request_started_at = graph.wait_for_throttle_window(cancel_flag).await?;
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

            graph.note_rate_limit_headers(response.headers());
            if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                if status == StatusCode::TOO_MANY_REQUESTS {
                    let _ = record_throttle_event(&graph.profile_id, UPLOAD_JOB_DIRECTION);
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
                report_upload_progress(graph, upload_job_id, transfer_id, offset, content_len);
                break;
            }

            if status.is_success() {
                forget_upload_session(&graph.profile_id, relative_path)?;
                let parsed = serde_json::from_str::<GraphDriveItem>(&text).map_err(|error| {
                    format!("Failed decoding upload-session completion response JSON: {error}")
                })?;
                report_upload_progress(graph, upload_job_id, transfer_id, content_len, content_len);
                if let Some(active_transfer_id) = transfer_id {
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

            // OneDrive refused the piece itself (an expired or broken
            // session): the next attempt starts a new one.
            if status.is_client_error() {
                forget_upload_session(&graph.profile_id, relative_path)?;
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

fn report_upload_progress(
    graph: &GraphContext,
    upload_job_id: i64,
    transfer_id: Option<&str>,
    bytes_done: u64,
    content_len: u64,
) {
    let _ = update_upload_job_progress(
        &graph.profile_id,
        upload_job_id,
        bytes_done,
        Some(content_len),
    );
    if let Some(active_transfer_id) = transfer_id {
        runtime_update_transfer_progress(
            &graph.sync_runtime,
            &graph.profile_id,
            active_transfer_id,
            bytes_done,
            Some(content_len),
        );
    }
}

/// Starts an upload session. `If-Match` is sent only here (never on chunk
/// uploads, where the eTag changes as the session runs).
pub(crate) async fn create_upload_session(
    graph: &mut GraphContext,
    relative_path: &str,
    target: &UploadTarget,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<UploadSessionResponse, String> {
    let encoded_path = encode_graph_path(relative_path);
    let url = format!(
        "{GRAPH_ROOT}/me/drive/root:/{}:/createUploadSession",
        encoded_path
    );
    let payload = serde_json::json!({
        "item": {
            "@microsoft.graph.conflictBehavior": target.conflict_behavior()
        }
    });
    let if_match = match target {
        UploadTarget::Replace { base_etag } => base_etag.clone(),
        UploadTarget::New => None,
    };
    let client = graph.http.clone();
    let mut refreshed = false;

    loop {
        ensure_not_cancelled(cancel_flag)?;
        let request_started_at = graph.wait_for_throttle_window(cancel_flag).await?;
        let access_token = graph.current_access_token().await;
        let response = tokio::select! {
            _ = wait_for_cancellation(Arc::clone(cancel_flag)) => {
                return Err(SYNC_CANCELLED_ERROR.to_string());
            }
            value = {
                let request = client.post(&url).bearer_auth(&access_token).json(&payload);
                let request = match if_match.as_deref() {
                    Some(etag) => request.header(reqwest::header::IF_MATCH, etag),
                    None => request,
                };
                request.send()
            } => {
                value.map_err(|error| format!("Create upload session request failed: {error}"))?
            }
        };

        let status = response.status();

        graph.note_rate_limit_headers(response.headers());
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
            let delay = parse_retry_after_delay(response.headers())
                .unwrap_or_else(|| Duration::from_secs(2));
            if status == StatusCode::TOO_MANY_REQUESTS {
                let _ = record_throttle_event(&graph.profile_id, UPLOAD_JOB_DIRECTION);
                graph.extend_throttle_window(delay, request_started_at)?;
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

        return serde_json::from_str::<UploadSessionResponse>(&text).map_err(|error| {
            format!("Failed decoding create upload session response JSON: {error}")
        });
    }
}

#[cfg(test)]
mod upload_session_tests {
    use super::*;

    fn entry(size: u64, mtime_ns: i64) -> LocalEntry {
        LocalEntry {
            is_dir: false,
            size,
            mtime_ns,
        }
    }

    #[test]
    fn a_saved_session_is_reused_only_for_the_same_unexpired_file() {
        let profile_id = test_profile_id("upload-session");
        let saved = SavedUploadSession {
            upload_url: "https://example.invalid/session".to_string(),
            local_size: 100,
            local_mtime_ns: 7,
            expires_at: current_unix_seconds() + 3600,
        };
        save_upload_session(&profile_id, "big.iso", &saved).expect("save");
        assert!(
            reusable_upload_session(&profile_id, "big.iso", &entry(100, 7))
                .expect("read")
                .is_some()
        );

        // The file changed: the session is dropped, not reused.
        assert!(
            reusable_upload_session(&profile_id, "big.iso", &entry(100, 8))
                .expect("read")
                .is_none()
        );
        assert!(read_saved_upload_session(&profile_id, "big.iso")
            .expect("read")
            .is_none());

        // About to expire: dropped too.
        save_upload_session(
            &profile_id,
            "big.iso",
            &SavedUploadSession {
                expires_at: current_unix_seconds() + 10,
                ..saved
            },
        )
        .expect("save");
        assert!(
            reusable_upload_session(&profile_id, "big.iso", &entry(100, 7))
                .expect("read")
                .is_none()
        );
    }

    #[test]
    fn an_unknown_expiry_counts_as_a_day() {
        let expiry = session_expiry(None);
        assert!(expiry > current_unix_seconds() + 23 * 3600);
        assert_eq!(
            session_expiry(Some("2026-11-06T10:00:00Z")),
            chrono::DateTime::parse_from_rfc3339("2026-11-06T10:00:00Z")
                .expect("date")
                .timestamp()
        );
    }
}
