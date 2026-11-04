use super::*;

// Repeats cloud renames and moves on this device (docs/sync-engine.md
// section 8a): the folder or file is renamed in place rather than
// downloaded again and deleted.

/// Carries out every queued local move. A move whose new path is already
/// taken is undone in the database instead (the planner then re-downloads
/// and removes the old copy through the delete guard); a move whose old path
/// is gone needs nothing on disk.
pub(crate) fn apply_pending_local_moves(
    profile_id: &str,
    account_prefix: &str,
    sync_root: &Path,
) -> Result<usize, String> {
    let mut applied = 0;
    for pending in list_pending_local_moves(profile_id)? {
        let old_abs = sync_root.join(path_to_local(&pending.old_path));
        let new_abs = sync_root.join(path_to_local(&pending.new_path));
        if read_local_entry(&old_abs)?.is_none() {
            finish_local_move(profile_id, &pending.old_path)?;
            continue;
        }
        if read_local_entry(&new_abs)?.is_some() {
            undo_local_follow(profile_id, &pending.old_path, &pending.new_path)?;
            log::warn!(
                "{} LOCAL_MOVE_BLOCKED from={} to={} reason=destination_exists",
                account_prefix,
                pending.old_path,
                pending.new_path
            );
            continue;
        }
        if let Some(parent) = new_abs.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("Failed creating '{}': {error}", parent.display()))?;
        }
        match std::fs::rename(&old_abs, &new_abs) {
            Ok(()) => {
                finish_local_move(profile_id, &pending.old_path)?;
                applied += 1;
                log::info!(
                    "{} LOCAL_MOVE from={} to={}",
                    account_prefix,
                    pending.old_path,
                    pending.new_path
                );
            }
            Err(error) => {
                undo_local_follow(profile_id, &pending.old_path, &pending.new_path)?;
                log::warn!(
                    "{} LOCAL_MOVE_FAILED from={} to={} error={}",
                    account_prefix,
                    pending.old_path,
                    pending.new_path,
                    error
                );
            }
        }
    }
    Ok(applied)
}
