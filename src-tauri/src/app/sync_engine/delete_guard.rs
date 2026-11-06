use super::*;

// The large-delete guard: the plan's deletes go through here, and a burst of
// deletes made on this computer bigger than the threshold waits for the
// user's review.

/// The guard adds up this computer's deletes over this window, so a folder a
/// file manager removes in pieces across several cycles counts as one burst.
pub(crate) const DELETE_GUARD_WINDOW_SECONDS: i64 = 5 * 60;

/// Applies the plan's deletes. Deletes made on this computer (going to the
/// cloud) are held for review when this cycle's batch plus those of the last
/// five minutes would remove more than the threshold (100 files by default).
/// An approval covers only paths that were on the reviewed list and are
/// still planned for deletion now.
///
/// Deletes made in the cloud (removed here, to the Trash) are never held and
/// do not count: OneDrive asked when they were made, and its recycle bin
/// keeps them. Deletes the user already confirmed inside SomeDrive (the
/// Clean up tab) are not held and do not count either; while other deletes
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
        .delete_remote
        .iter()
        .map(|row| row.path.clone())
        .partition(|path| user_approved.contains(path));
    if guarded_paths.is_empty() {
        // Nothing is held, so no large-deletion warning may remain, however
        // the earlier review ended.
        let warning_shown = read_persisted_sync_issue(&graph.profile_id)?
            .is_some_and(|issue| issue.issue_code == "large_delete_guard");
        if !guard_state.pending_paths.is_empty() || warning_shown {
            persist_large_delete_guard_state(&graph.profile_id, &LargeDeleteGuardState::default())?;
            runtime_clear_issue(&graph.sync_runtime, &graph.profile_id);
            guard_state = LargeDeleteGuardState::default();
        }
    }
    if guarded_paths.is_empty() && approved_paths.is_empty() && planned.delete_local.is_empty() {
        return Ok(());
    }

    let threshold = resolve_large_delete_guard_threshold();
    let recent_deletes = count_events_since(
        &graph.profile_id,
        DELETE_EVENT_DIRECTION,
        current_unix_seconds() - DELETE_GUARD_WINDOW_SECONDS,
    )?;
    let within_limit = is_within_delete_limit(guarded_paths.len(), recent_deletes, threshold);

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
            "{} [cycle:{}] LARGE_DELETE_GUARD_HELD planned={} recent_window={} window_seconds={} threshold={} user_approved_going_ahead={}",
            graph.account_prefix,
            graph.cycle_id,
            guarded_paths.len(),
            recent_deletes,
            DELETE_GUARD_WINDOW_SECONDS,
            threshold,
            approved_paths.len()
        );
        runtime_set_issue(
            &graph.sync_runtime,
            &graph.profile_id,
            "large_delete_guard",
            &large_delete_guard_issue_message(guarded_paths.len(), recent_deletes),
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

    let confirmed_by_review =
        !held && guard_state.approved && !(guarded_paths.is_empty() || within_limit);
    let user_confirmed =
        user_confirmed_deletes(user_approved, &guarded_paths, &allowed, confirmed_by_review);

    // Deletes made in the cloud are never held.
    let mut delete_local: Vec<SyncFileRow> = planned.delete_local.clone();
    let delete_remote: Vec<SyncFileRow> = planned
        .delete_remote
        .iter()
        .filter(|row| allowed.contains(&row.path))
        .cloned()
        .collect();
    // Deepest first, so a folder is only reached after its contents.
    delete_local.sort_by_key(|row| std::cmp::Reverse(row.path.matches('/').count()));
    delete_local_items(graph, sync_root, &delete_local, stats)?;
    apply_remote_deletes(graph, &delete_remote, &user_confirmed, stats, cancel_flag).await?;

    if !held && (guard_state.approved || !guard_state.pending_paths.is_empty()) {
        // The held deletes went ahead (confirmed, or now within the limit):
        // the review and its warning end together.
        persist_large_delete_guard_state(&graph.profile_id, &LargeDeleteGuardState::default())?;
        runtime_clear_issue(&graph.sync_runtime, &graph.profile_id);
    }
    Ok(())
}

/// Whether this batch, with the deletes of the guard's window, stays within
/// the threshold. A batch over the threshold on its own is held even with no
/// recent deletes.
fn is_within_delete_limit(batch: usize, recent_deletes: usize, threshold: usize) -> bool {
    batch + recent_deletes <= threshold
}

/// The warning names the batch and, when earlier deletes pushed it over the
/// threshold, those too, so a small batch held because of them makes sense.
pub(crate) fn large_delete_guard_issue_message(
    pending_count: usize,
    recent_deletes: usize,
) -> String {
    let window_minutes = DELETE_GUARD_WINDOW_SECONDS / 60;
    if recent_deletes == 0 {
        format!(
            "{pending_count} files would be deleted from OneDrive. Review them, then confirm the deletion or keep the files."
        )
    } else {
        format!(
            "{pending_count} files would be deleted from OneDrive, after {recent_deletes} deleted in the last {window_minutes} minutes. Review them, then confirm the deletion or keep the files."
        )
    }
}

/// The deletes this cycle that do not count towards the guard's window: those the user confirmed in Clean up, and, when the user approved
/// the review, the reviewed deletes now going ahead. Confirming one large
/// deletion must not hold the deletes that follow it; deletes nobody
/// confirmed still count.
fn user_confirmed_deletes(
    clean_up_approved: HashSet<String>,
    guarded_paths: &[String],
    allowed: &HashSet<String>,
    confirmed_by_review: bool,
) -> HashSet<String> {
    let mut confirmed = clean_up_approved;
    if confirmed_by_review {
        confirmed.extend(
            guarded_paths
                .iter()
                .filter(|path| allowed.contains(*path))
                .cloned(),
        );
    }
    confirmed
}

#[cfg(test)]
mod delete_guard_tests {
    use super::*;

    fn paths(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    fn deleted_row(path: String) -> SyncFileRow {
        SyncFileRow {
            path,
            is_dir: false,
            remote_item_id: None,
            cloud_present: false,
            cloud_size: 0,
            cloud_mtime: 0,
            cloud_etag: None,
            cloud_ctag: None,
            cloud_hash: None,
            cloud_download_url: None,
            cloud_download_url_at: 0,
            base_present: true,
            base_size: 1,
            base_local_mtime_ns: 0,
            base_etag: None,
            base_hash: None,
            base_cloud_size: 1,
            desired_action: String::new(),
        }
    }

    fn rows(prefix: &str, count: usize) -> Vec<SyncFileRow> {
        (0..count)
            .map(|index| deleted_row(format!("{prefix}/file-{index}.txt")))
            .collect()
    }

    fn run_guard(graph: &mut GraphContext, planned: &PlannedActions) {
        let sync_root = std::env::temp_dir().join(format!("somedrive-guard-{}", graph.profile_id));
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let mut stats = SyncCycleStats::default();
        tauri::async_runtime::block_on(apply_deletes_through_guard(
            graph,
            &sync_root,
            planned,
            &mut stats,
            &cancel_flag,
        ))
        .expect("guard");
    }

    fn recent_delete_events(profile_id: &str) -> usize {
        count_events_since(
            profile_id,
            DELETE_EVENT_DIRECTION,
            current_unix_seconds() - DELETE_GUARD_WINDOW_SECONDS,
        )
        .expect("count")
    }

    #[test]
    fn the_limit_counts_the_batch_and_the_window_together() {
        let threshold = 100;
        assert!(is_within_delete_limit(100, 0, threshold));
        assert!(
            !is_within_delete_limit(101, 0, threshold),
            "one large batch"
        );
        assert!(is_within_delete_limit(60, 40, threshold));
        assert!(
            !is_within_delete_limit(61, 40, threshold),
            "a folder deleted in pieces adds up within the window"
        );
    }

    #[test]
    fn the_warning_names_earlier_deletes_only_when_they_count() {
        assert_eq!(
            large_delete_guard_issue_message(121, 0),
            "121 files would be deleted from OneDrive. Review them, then confirm the deletion or keep the files."
        );
        assert_eq!(
            large_delete_guard_issue_message(30, 90),
            "30 files would be deleted from OneDrive, after 90 deleted in the last 5 minutes. Review them, then confirm the deletion or keep the files."
        );
    }

    #[test]
    fn a_large_delete_made_on_this_computer_is_held() {
        let mut graph = test_graph_context("guard-held");
        let planned = PlannedActions {
            delete_remote: rows("Data", 101),
            ..PlannedActions::default()
        };
        run_guard(&mut graph, &planned);

        let guard_state = read_large_delete_guard_state(&graph.profile_id).expect("state");
        assert_eq!(guard_state.pending_paths.len(), 101);
        assert!(!guard_state.approved);
        assert_eq!(
            recent_delete_events(&graph.profile_id),
            0,
            "nothing was deleted"
        );
    }

    #[test]
    fn deletes_made_in_the_cloud_are_never_held_and_never_counted() {
        let mut graph = test_graph_context("guard-cloud");
        let planned = PlannedActions {
            delete_local: rows("Cloud", 500),
            ..PlannedActions::default()
        };
        run_guard(&mut graph, &planned);

        let guard_state = read_large_delete_guard_state(&graph.profile_id).expect("state");
        assert!(guard_state.pending_paths.is_empty());
        assert_eq!(recent_delete_events(&graph.profile_id), 0);
    }

    #[test]
    fn deletes_made_in_the_cloud_go_ahead_while_local_deletes_are_held() {
        let mut graph = test_graph_context("guard-mixed");
        let planned = PlannedActions {
            delete_local: rows("Cloud", 50),
            delete_remote: rows("Data", 101),
            ..PlannedActions::default()
        };
        run_guard(&mut graph, &planned);

        let guard_state = read_large_delete_guard_state(&graph.profile_id).expect("state");
        assert_eq!(
            guard_state.pending_paths.len(),
            101,
            "only this computer's deletes are on the review list"
        );
        assert!(guard_state
            .pending_paths
            .iter()
            .all(|path| path.starts_with("Data/")));
    }

    #[test]
    fn reviewed_and_confirmed_deletes_do_not_count_towards_the_window() {
        let guarded = paths(&["Games/a.gba", "Games/b.gba", "Games/new.gba"]);
        // The review covered a and b; new.gba appeared after it.
        let allowed: HashSet<String> = paths(&["Games/a.gba", "Games/b.gba"]).into_iter().collect();
        let clean_up: HashSet<String> = paths(&["Pictures/x.jpg"]).into_iter().collect();

        let confirmed = user_confirmed_deletes(clean_up.clone(), &guarded, &allowed, true);
        assert!(confirmed.contains("Games/a.gba"));
        assert!(confirmed.contains("Pictures/x.jpg"));
        assert!(
            !confirmed.contains("Games/new.gba"),
            "not on the reviewed list: it was not confirmed"
        );

        // Within the limit (no review): only Clean up approvals are exempt.
        let unreviewed = user_confirmed_deletes(clean_up, &guarded, &allowed, false);
        assert_eq!(unreviewed, paths(&["Pictures/x.jpg"]).into_iter().collect());
    }
}
