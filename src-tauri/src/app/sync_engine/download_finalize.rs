use super::*;

// The last steps of a download: check what arrived, give it OneDrive's time,
// and move it into place without overwriting a local change.

/// A download is only accepted when its size matches what the server said it
/// was sending (`Content-Length`, or the planned size without one; SharePoint
/// sizes can differ from the listed size) and, when OneDrive supplied one,
/// its QuickXorHash matches.
pub(crate) fn verify_downloaded_content(
    version: &CloudVersion,
    content_length: Option<u64>,
    downloaded_bytes: u64,
    hasher: &QuickXorHasher,
) -> Result<(), String> {
    let expected_size = content_length.unwrap_or(version.size);
    if downloaded_bytes != expected_size {
        return Err(format!(
            "Downloaded size {downloaded_bytes} does not match the expected {expected_size} bytes"
        ));
    }
    if let Some(expected) = version.quick_xor_hash.as_deref() {
        let actual = hasher.finish_base64();
        if actual != expected {
            return Err(format!(
                "Downloaded content hash {actual} does not match OneDrive's {expected}"
            ));
        }
    }
    Ok(())
}

pub(crate) fn set_file_modified_seconds(file: &std::fs::File, unix_seconds: i64) {
    if unix_seconds <= 0 {
        return;
    }
    let modified = std::time::UNIX_EPOCH + Duration::from_secs(unix_seconds as u64);
    if let Err(error) = file.set_modified(modified) {
        log::warn!("DOWNLOAD_SET_MTIME_FAILED error={}", error);
    }
}

/// Moves a verified download into place. The file currently at the target is
/// only replaced when it still matches the baseline (or the plan expected
/// nothing there). Otherwise the download is discarded and the path is held
/// as a conflict (`None`), so a local edit made since the plan is never lost
/// or duplicated.
pub(crate) fn place_downloaded_file(
    graph: &GraphContext,
    relative_path: &str,
    temp_path: &Path,
    local_path: &Path,
) -> Result<Option<LocalEntry>, String> {
    if let Some(existing) = read_local_entry(local_path)? {
        if existing.is_dir {
            return Err(format!(
                "A folder is in the way of '{relative_path}'; the file is not downloaded"
            ));
        }
        let row = read_sync_file_row(&graph.profile_id, relative_path)?;
        let replaceable = row
            .as_ref()
            .is_some_and(|row| row.local_matches_baseline(&existing));
        if !replaceable && same_content(local_path, temp_path)? {
            // The file in the way already holds exactly the downloaded
            // content (it finished just before a restart, say): keep it and
            // record it as synced instead of raising a conflict.
            let _ = std::fs::remove_file(temp_path);
            log::info!(
                "{} [cycle:{}] DOWNLOAD_ALREADY_PRESENT path={}",
                graph.account_prefix,
                graph.cycle_id,
                relative_path
            );
            return Ok(Some(existing));
        }
        if !replaceable {
            let _ = std::fs::remove_file(temp_path);
            let cloud_mtime = row.map(|row| row.cloud_mtime).unwrap_or(0);
            hold_conflict(
                graph,
                relative_path,
                ISSUE_CONFLICT_BOTH_CHANGED,
                existing.mtime_ns,
                cloud_mtime,
            )
            .map_err(|error| error.to_string())?;
            return Ok(None);
        }
    }
    std::fs::rename(temp_path, local_path).map_err(|error| {
        format!(
            "Failed moving '{}' to '{}': {error}",
            temp_path.display(),
            local_path.display()
        )
    })?;
    if let Some(parent) = local_path.parent() {
        graph.note_dir_needs_sync(parent);
    }
    read_local_entry(local_path)?
        .map(Some)
        .ok_or_else(|| format!("'{relative_path}' is missing right after it was downloaded"))
}

/// Whether two files hold the same bytes (size first, then QuickXorHash).
fn same_content(existing: &Path, downloaded: &Path) -> Result<bool, String> {
    let existing_size = std::fs::metadata(existing)
        .map_err(|error| format!("Failed reading '{}': {error}", existing.display()))?
        .len();
    let downloaded_size = std::fs::metadata(downloaded)
        .map_err(|error| format!("Failed reading '{}': {error}", downloaded.display()))?
        .len();
    if existing_size != downloaded_size {
        return Ok(false);
    }
    Ok(quick_xor_hash_file(existing)? == quick_xor_hash_file(downloaded)?)
}

/// What finishing a downloaded file asks of the download loop.
pub(crate) enum FinishStep {
    Done(RemoteDownloadOutcome),
    Retry,
}

/// Puts a verified download in place: sets OneDrive's modified time, then
/// moves it over the target only if that is still safe (see
/// `place_downloaded_file`).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn finish_downloaded_file(
    graph: &GraphContext,
    job_id: Option<i64>,
    transfer_id: &Option<String>,
    attempt: u32,
    item_id: &str,
    relative_path: &str,
    temp_path: &Path,
    local_path: &Path,
    cloud_mtime: i64,
    downloaded_bytes: u64,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<FinishStep, String> {
    // OneDrive's time, so both sides agree on when the file changed.
    match std::fs::OpenOptions::new().write(true).open(temp_path) {
        Ok(file) => set_file_modified_seconds(&file, cloud_mtime),
        Err(error) => log::warn!("DOWNLOAD_SET_MTIME_OPEN_FAILED error={}", error),
    }

    if cancel_flag.load(Ordering::Relaxed) {
        runtime_finish_transfer_cancelled(&graph.sync_runtime, &graph.profile_id, transfer_id);
        let _ = std::fs::remove_file(temp_path);
        return Err(SYNC_CANCELLED_ERROR.to_string());
    }

    let placed_entry = match place_downloaded_file(graph, relative_path, temp_path, local_path) {
        Ok(Some(entry)) => entry,
        Ok(None) => {
            runtime_finish_transfer_cancelled(&graph.sync_runtime, &graph.profile_id, transfer_id);
            return Ok(FinishStep::Done(RemoteDownloadOutcome::HeldAsConflict));
        }
        Err(reason) => {
            if handle_download_retry(
                graph,
                job_id,
                cancel_flag,
                transfer_id,
                attempt,
                relative_path,
                &reason,
                Some(temp_path),
                &reason,
            )
            .await?
            {
                return Ok(FinishStep::Retry);
            }
            return Err(reason);
        }
    };

    runtime_finish_transfer_download_success(
        &graph.sync_runtime,
        &graph.profile_id,
        transfer_id,
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
    Ok(FinishStep::Done(RemoteDownloadOutcome::Downloaded(
        placed_entry,
    )))
}

/// Microsoft answered 429: the whole account waits `delay`, the download
/// limits are lowered, and this job goes back to the queue until then.
/// Returns the error the download ends with.
pub(crate) fn defer_download_after_throttle(
    graph: &GraphContext,
    job_id: Option<i64>,
    transfer_id: &Option<String>,
    relative_path: &str,
    status: StatusCode,
    delay: Duration,
    request_started_at: std::time::Instant,
) -> Result<String, String> {
    let _ = record_throttle_event(&graph.profile_id, DOWNLOAD_JOB_DIRECTION);
    graph.extend_throttle_window(delay, request_started_at)?;
    if let Some(active_job_id) = job_id {
        let retry_reason = format!("Download retry scheduled for HTTP status {status}");
        if let Err(error) =
            mark_download_job_retry_wait(&graph.profile_id, active_job_id, &retry_reason, delay)
        {
            log::warn!(
                "{} [cycle:{}] DOWNLOAD_RETRY_WAIT_PERSIST_FAILED path={} error={}",
                graph.account_prefix,
                graph.cycle_id,
                relative_path,
                error
            );
        }
    }
    runtime_finish_transfer_cancelled(&graph.sync_runtime, &graph.profile_id, transfer_id);
    log::warn!(
        "{} [cycle:{}] DOWNLOAD_RETRY_DEFERRED status={} path={} delay_ms={}",
        graph.account_prefix,
        graph.cycle_id,
        status,
        relative_path,
        delay.as_millis()
    );
    Ok(DOWNLOAD_RETRY_DEFERRED_ERROR.to_string())
}

#[cfg(test)]
mod same_content_tests {
    use super::*;

    #[test]
    fn identical_bytes_match_and_different_bytes_do_not() {
        let folder = std::env::temp_dir().join(format!(
            "somedrive-same-content-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&folder).expect("folder");
        let first = folder.join("first.bin");
        let second = folder.join("second.bin");
        let third = folder.join("third.bin");
        std::fs::write(&first, b"same bytes").expect("write");
        std::fs::write(&second, b"same bytes").expect("write");
        std::fs::write(&third, b"diff bytes").expect("write");
        assert!(same_content(&first, &second).expect("compare"));
        assert!(!same_content(&first, &third).expect("compare"));
        let _ = std::fs::remove_dir_all(&folder);
    }
}
