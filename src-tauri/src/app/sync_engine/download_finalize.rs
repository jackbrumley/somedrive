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
