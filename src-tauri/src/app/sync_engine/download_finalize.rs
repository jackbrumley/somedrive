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
