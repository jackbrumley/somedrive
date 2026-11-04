use super::*;

// Compares local files with their cloud versions by content, on every CPU
// core at once. A first sync on a drive already full of files hashes tens
// of gigabytes; one file at a time left all but one core idle.

/// What the content check found for one row.
pub(crate) enum ContentCheck {
    /// Same content as the cloud version; `entry` is the file as hashed.
    Matches(LocalEntry),
    /// Different content (or no hash to prove it equal).
    Differs(LocalEntry),
    /// A folder where the cloud has a file, or the other way round.
    KindMismatch,
    /// Gone, or changed while it was being hashed: the next cycle plans it
    /// again from what is there then.
    Skip,
}

/// Whether the local file's content equals the cloud version in `row`.
/// Empty files match without a hash; a non-empty file is never assumed equal
/// without one.
pub(crate) fn local_content_matches_cloud(
    row: &SyncFileRow,
    local_path: &Path,
    entry: &LocalEntry,
) -> Result<bool, String> {
    if entry.size == 0 && row.cloud_size == 0 {
        return Ok(true);
    }
    if entry.size != row.cloud_size {
        return Ok(false);
    }
    match row.cloud_hash.as_deref() {
        Some(cloud_hash) => Ok(quick_xor_hash_file(local_path)? == cloud_hash),
        None => Ok(false),
    }
}

/// Checks one row: reads the file, compares it, and reads it again so a
/// file that changed while it was hashed is skipped rather than recorded.
pub(crate) fn check_local_content(
    row: &SyncFileRow,
    local_path: &Path,
) -> Result<ContentCheck, String> {
    let Some(entry) = read_local_entry(local_path)? else {
        return Ok(ContentCheck::Skip);
    };
    if row.is_dir || entry.is_dir {
        // A folder on both sides only needs its baseline.
        return Ok(if row.is_dir && entry.is_dir {
            ContentCheck::Matches(entry)
        } else {
            ContentCheck::KindMismatch
        });
    }
    let matches = local_content_matches_cloud(row, local_path, &entry)?;
    if read_local_entry(local_path)?.as_ref() != Some(&entry) {
        return Ok(ContentCheck::Skip);
    }
    Ok(if matches {
        ContentCheck::Matches(entry)
    } else {
        ContentCheck::Differs(entry)
    })
}

/// How many files are hashed at once: every core the machine offers.
pub(crate) fn content_check_threads() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(4)
}

/// Checks `rows` on `content_check_threads()` worker threads and sends each
/// result (with the row's index) as soon as it is ready. Workers stop when
/// `cancel_flag` is set or the receiver is dropped.
pub(crate) fn spawn_content_checks(
    rows: Vec<SyncFileRow>,
    sync_root: PathBuf,
    cancel_flag: Arc<AtomicBool>,
) -> mpsc::Receiver<(usize, Result<ContentCheck, String>)> {
    let threads = content_check_threads().min(rows.len().max(1));
    let (result_tx, result_rx) = mpsc::channel(threads * 4);
    tauri::async_runtime::spawn_blocking(move || {
        let next = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..threads {
                let result_tx = result_tx.clone();
                let (rows, next, sync_root, cancel_flag) = (&rows, &next, &sync_root, &cancel_flag);
                scope.spawn(move || loop {
                    if cancel_flag.load(Ordering::Relaxed) {
                        return;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(row) = rows.get(index) else {
                        return;
                    };
                    let local_path = local_path_for(sync_root, &row.path);
                    let result = check_local_content(row, &local_path);
                    if result_tx.blocking_send((index, result)).is_err() {
                        return;
                    }
                });
            }
        });
    });
    result_rx
}

#[cfg(test)]
mod content_check_tests {
    use super::*;

    fn row_for(path: &str, size: u64, hash: Option<String>) -> SyncFileRow {
        SyncFileRow {
            path: path.to_string(),
            is_dir: false,
            remote_item_id: Some(format!("id-{path}")),
            cloud_present: true,
            cloud_size: size,
            cloud_mtime: 0,
            cloud_etag: None,
            cloud_ctag: None,
            cloud_hash: hash,
            cloud_download_url: None,
            cloud_download_url_at: 0,
            base_present: false,
            base_size: 0,
            base_local_mtime_ns: 0,
            base_etag: None,
            base_hash: None,
            base_cloud_size: 0,
            desired_action: PLANNER_ACTION_VERIFY.to_string(),
        }
    }

    #[test]
    fn every_file_is_checked_in_parallel_with_the_right_verdict() {
        let root = std::env::temp_dir().join(format!(
            "somedrive-content-check-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).expect("root");
        let mut rows = Vec::new();
        for index in 0..40 {
            let name = format!("file-{index}.txt");
            let body = format!("content {index}");
            std::fs::write(root.join(&name), &body).expect("write");
            let mut hasher = QuickXorHasher::new();
            hasher.update(body.as_bytes());
            // Odd files carry another file's hash, so they differ.
            let hash = if index % 2 == 0 {
                hasher.finish_base64()
            } else {
                "not-this-content".to_string()
            };
            rows.push(row_for(&name, body.len() as u64, Some(hash)));
        }
        rows.push(row_for("missing.txt", 3, None));

        let mut receiver =
            spawn_content_checks(rows.clone(), root.clone(), Arc::new(AtomicBool::new(false)));
        let mut verdicts = vec![None; rows.len()];
        tauri::async_runtime::block_on(async {
            while let Some((index, result)) = receiver.recv().await {
                verdicts[index] = Some(match result.expect("check") {
                    ContentCheck::Matches(_) => "matches",
                    ContentCheck::Differs(_) => "differs",
                    ContentCheck::KindMismatch => "kind",
                    ContentCheck::Skip => "skip",
                });
            }
        });
        for (index, verdict) in verdicts.iter().enumerate().take(40) {
            let expected = if index % 2 == 0 { "matches" } else { "differs" };
            assert_eq!(*verdict, Some(expected), "file {index}");
        }
        assert_eq!(verdicts[40], Some("skip"), "a missing file is skipped");
        let _ = std::fs::remove_dir_all(&root);
    }
}
