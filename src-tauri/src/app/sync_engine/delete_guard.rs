use super::*;

// The large-delete guard: the plan's deletes go through here, and a batch
// bigger than the threshold waits for the user's review.

pub(crate) const DELETE_GUARD_WINDOW_SECONDS: i64 = 24 * 60 * 60;

/// Applies the plan's deletes, holding them for review when a cycle (with
/// the last 24 hours of deletes) would remove more than the threshold: 10
/// files or 10% of synced files, whichever is larger, in either direction.
/// An approval covers only paths that were on the reviewed list and are
/// still planned for deletion now.
///
/// Deletes the user already confirmed inside SomeDrive (the Clean up tab)
/// are not held and do not count towards the threshold; while other deletes
/// are held for review, these still go ahead.
pub(crate) async fn apply_deletes_through_guard(
    graph: &mut GraphContext,
    sync_root: &Path,
    planned: &PlannedActions,
    stats: &mut SyncCycleStats,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<()> {
    let mut guard_state = read_large_delete_guard_state(&graph.profile_id)?;
    settle_approved_deletes(&graph.profile_id)?;
    let user_approved = read_approved_deletes(&graph.profile_id)?;
    let (approved_paths, guarded_paths): (Vec<String>, Vec<String>) = planned
        .delete_local
        .iter()
        .chain(planned.delete_remote.iter())
        .map(|row| row.path.clone())
        .partition(|path| user_approved.contains(path));
    if guarded_paths.is_empty() && !guard_state.pending_paths.is_empty() {
        persist_large_delete_guard_state(&graph.profile_id, &LargeDeleteGuardState::default())?;
        runtime_clear_issue(&graph.sync_runtime, &graph.profile_id);
        guard_state = LargeDeleteGuardState::default();
    }
    if guarded_paths.is_empty() && approved_paths.is_empty() {
        return Ok(());
    }

    let tracked_files = count_tracked_files(&graph.profile_id)?;
    let threshold = resolve_large_delete_guard_threshold().max(tracked_files / 10);
    let recent_deletes = count_events_since(
        &graph.profile_id,
        DELETE_EVENT_DIRECTION,
        current_unix_seconds() - DELETE_GUARD_WINDOW_SECONDS,
    )?;
    let within_limit = guarded_paths.len() + recent_deletes <= threshold;

    let mut held = false;
    let mut allowed: HashSet<String> = if guarded_paths.is_empty() || within_limit {
        guarded_paths.iter().cloned().collect()
    } else if guard_state.approved {
        let reviewed: HashSet<&String> = guard_state.pending_paths.iter().collect();
        guarded_paths
            .iter()
            .filter(|path| reviewed.contains(path))
            .cloned()
            .collect()
    } else {
        held = true;
        guard_state.pending_paths = guarded_paths.clone();
        persist_large_delete_guard_state(&graph.profile_id, &guard_state)?;
        log::warn!(
            "{} [cycle:{}] LARGE_DELETE_GUARD_HELD planned={} recent_24h={} threshold={} user_approved_going_ahead={}",
            graph.account_prefix,
            graph.cycle_id,
            guarded_paths.len(),
            recent_deletes,
            threshold,
            approved_paths.len()
        );
        runtime_set_issue(
            &graph.sync_runtime,
            &graph.profile_id,
            "large_delete_guard",
            &large_delete_guard_issue_message(guarded_paths.len()),
            &["confirm_large_delete", "keep_cloud_files", "open_sync_root"],
            None,
            None,
        );
        HashSet::new()
    };
    if !approved_paths.is_empty() {
        log::info!(
            "{} [cycle:{}] USER_APPROVED_DELETES_APPLYING count={}",
            graph.account_prefix,
            graph.cycle_id,
            approved_paths.len()
        );
    }
    allowed.extend(approved_paths);

    let mut delete_local: Vec<SyncFileRow> = planned
        .delete_local
        .iter()
        .filter(|row| allowed.contains(&row.path))
        .cloned()
        .collect();
    let delete_remote: Vec<SyncFileRow> = planned
        .delete_remote
        .iter()
        .filter(|row| allowed.contains(&row.path))
        .cloned()
        .collect();
    // Deepest first, so a folder is only reached after its contents.
    delete_local.sort_by_key(|row| std::cmp::Reverse(row.path.matches('/').count()));
    delete_local_items(graph, sync_root, &delete_local, stats)?;
    apply_remote_deletes(graph, &delete_remote, &user_approved, stats, cancel_flag).await?;

    if !held && (guard_state.approved || !guard_state.pending_paths.is_empty()) {
        persist_large_delete_guard_state(&graph.profile_id, &LargeDeleteGuardState::default())?;
    }
    Ok(())
}

pub(crate) fn large_delete_guard_issue_message(pending_count: usize) -> String {
    format!(
        "{pending_count} files would be deleted. Review them, then confirm the deletion or keep the files."
    )
}
