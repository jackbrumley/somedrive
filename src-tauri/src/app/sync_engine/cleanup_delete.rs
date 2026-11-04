use super::*;

// Deleting from the Clean up tab, after the user typed the confirmation.
// The suggestion is worked out again from the database, every file is
// compared byte for byte with the copy that stays, and only then do the
// files go to the Trash here. The sync then deletes them in OneDrive (its
// recycle bin keeps them) without asking again.

const COMPARE_BUFFER_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupDeleteResult {
    pub file_count: usize,
    pub bytes: u64,
}

/// Deletes a duplicate suggestion. `suggestion_id` must still match what
/// the analysis suggests now; otherwise nothing is deleted.
pub fn delete_duplicate_suggestion(
    profile_id: &str,
    sync_root: &Path,
    suggestion_id: &str,
) -> Result<CleanupDeleteResult, String> {
    let view = list_cleanup(profile_id)?;
    let suggestion = view
        .duplicates
        .into_iter()
        .find(|suggestion| suggestion.id == suggestion_id)
        .ok_or_else(|| {
            "This suggestion changed since it was shown, so nothing was deleted. Check the refreshed list and try again.".to_string()
        })?;
    log::info!(
        "{} CLEANUP_DELETE_START kind={} path={} files={} bytes={}",
        log_context::account_prefix(profile_id),
        suggestion.kind,
        suggestion.path,
        suggestion.file_count,
        suggestion.bytes
    );

    verify_identical_on_disk(sync_root, &suggestion.all_pairs)?;
    let expected: HashSet<&str> = suggestion
        .all_pairs
        .iter()
        .map(|pair| pair.path.as_str())
        .collect();
    for path in &suggestion.delete_paths {
        ensure_nothing_else_inside(sync_root, path, &expected)?;
    }

    trash_approved(profile_id, sync_root, &suggestion.delete_paths)?;
    log::info!(
        "{} CLEANUP_DELETE_DONE path={} files={} bytes={}",
        log_context::account_prefix(profile_id),
        suggestion.path,
        suggestion.file_count,
        suggestion.bytes
    );
    Ok(CleanupDeleteResult {
        file_count: suggestion.file_count,
        bytes: suggestion.bytes,
    })
}

/// Deletes one file from the Large files list.
pub fn delete_large_file(
    profile_id: &str,
    sync_root: &Path,
    path: &str,
) -> Result<CleanupDeleteResult, String> {
    let view = list_cleanup(profile_id)?;
    let file = view
        .large_files
        .into_iter()
        .find(|file| file.path == path)
        .ok_or_else(|| {
            format!("'{path}' is no longer in the Large files list, so it was not deleted.")
        })?;
    let local_path = local_path_for(sync_root, &file.path);
    let on_disk = std::fs::metadata(&local_path)
        .map_err(|error| format!("Could not read '{}': {error}", local_path.display()))?;
    if on_disk.len() != file.size {
        return Err(format!(
            "'{path}' changed on this device since it was synced, so it was not deleted."
        ));
    }
    trash_approved(profile_id, sync_root, std::slice::from_ref(&file.path))?;
    log::info!(
        "{} CLEANUP_LARGE_FILE_DELETED path={} bytes={} copies={}",
        log_context::account_prefix(profile_id),
        file.path,
        file.size,
        file.copies.len()
    );
    Ok(CleanupDeleteResult {
        file_count: 1,
        bytes: file.size,
    })
}

/// Approves the deletes for the sync (so the large-delete guard does not
/// ask again), moves each path to the Trash and records it gone here.
fn trash_approved(profile_id: &str, sync_root: &Path, paths: &[String]) -> Result<(), String> {
    let mut approved = Vec::new();
    for path in paths {
        approved.extend(list_tracked_paths_at_or_inside(profile_id, path)?);
    }
    approve_deletes(profile_id, &approved)?;
    for path in paths {
        let local_path = local_path_for(sync_root, path);
        trash::delete(&local_path).map_err(|error| {
            format!(
                "Failed moving '{}' to the Trash: {error}",
                local_path.display()
            )
        })?;
        record_local_tree_absent(profile_id, path)?;
        log::info!(
            "{} CLEANUP_TRASHED path={}",
            log_context::account_prefix(profile_id),
            path
        );
    }
    Ok(())
}

/// Compares each file with the copy that stays, byte for byte, on every
/// core. Any difference (or a file that cannot be read) stops the delete.
pub(crate) fn verify_identical_on_disk(
    sync_root: &Path,
    pairs: &[DuplicatePair],
) -> Result<(), String> {
    let next = AtomicUsize::new(0);
    let failure: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    let threads = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4)
        .min(pairs.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(pair) = pairs.get(index) else {
                    break;
                };
                if failure.lock().map(|found| found.is_some()).unwrap_or(true) {
                    break;
                }
                let outcome = files_identical(
                    &local_path_for(sync_root, &pair.path),
                    &local_path_for(sync_root, &pair.kept_path),
                );
                let problem = match outcome {
                    Ok(true) => None,
                    Ok(false) => Some(format!(
                        "'{}' is not identical to '{}' on this device, so nothing was deleted.",
                        pair.path, pair.kept_path
                    )),
                    Err(error) => Some(format!(
                        "Could not compare '{}' with '{}': {error}. Nothing was deleted.",
                        pair.path, pair.kept_path
                    )),
                };
                if let Some(problem) = problem {
                    if let Ok(mut found) = failure.lock() {
                        found.get_or_insert(problem);
                    }
                    break;
                }
            });
        }
    });
    match failure.into_inner() {
        Ok(None) => Ok(()),
        Ok(Some(problem)) => Err(problem),
        Err(_) => Err("The file comparison failed unexpectedly. Nothing was deleted.".to_string()),
    }
}

pub(crate) fn files_identical(left: &Path, right: &Path) -> std::io::Result<bool> {
    let mut left_file = std::fs::File::open(left)?;
    let mut right_file = std::fs::File::open(right)?;
    if left_file.metadata()?.len() != right_file.metadata()?.len() {
        return Ok(false);
    }
    let mut left_buffer = vec![0u8; COMPARE_BUFFER_BYTES];
    let mut right_buffer = vec![0u8; COMPARE_BUFFER_BYTES];
    loop {
        let read = read_full(&mut left_file, &mut left_buffer)?;
        let right_read = read_full(&mut right_file, &mut right_buffer)?;
        if read != right_read || left_buffer[..read] != right_buffer[..read] {
            return Ok(false);
        }
        if read == 0 {
            return Ok(true);
        }
    }
}

/// Reads until `buffer` is full or the file ends.
fn read_full(file: &mut std::fs::File, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        let read = file.read(&mut buffer[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}

/// A folder is only deleted whole when every file in it on disk is one of
/// the checked copies: a new or unsynced file stops the delete.
fn ensure_nothing_else_inside(
    sync_root: &Path,
    path: &str,
    expected: &HashSet<&str>,
) -> Result<(), String> {
    let local_path = local_path_for(sync_root, path);
    let metadata = std::fs::symlink_metadata(&local_path)
        .map_err(|error| format!("Could not read '{}': {error}", local_path.display()))?;
    if !metadata.is_dir() {
        return Ok(());
    }
    let mut pending = vec![local_path];
    while let Some(folder) = pending.pop() {
        let entries = std::fs::read_dir(&folder)
            .map_err(|error| format!("Could not list '{}': {error}", folder.display()))?;
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("Could not list '{}': {error}", folder.display()))?;
            let file_type = entry
                .file_type()
                .map_err(|error| format!("Could not read '{}': {error}", entry.path().display()))?;
            if file_type.is_dir() {
                pending.push(entry.path());
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(sync_root)
                .map(|relative| normalize_relative_path(&relative.to_string_lossy()))
                .unwrap_or_default();
            if !expected.contains(relative.as_str()) {
                return Err(format!(
                    "'{path}' holds '{relative}', which is not a checked copy (it may be new or not synced yet), so nothing was deleted."
                ));
            }
        }
    }
    Ok(())
}
