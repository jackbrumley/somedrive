// Paths: Graph path encoding, the local scan, and local name rules.

fn split_parent_and_name(relative_path: &str) -> Result<(&str, &str), String> {
    let trimmed = relative_path.trim_matches('/');
    if trimmed.is_empty() {
        return Err("Remote folder path is empty".to_string());
    }
    if let Some((parent, name)) = trimmed.rsplit_once('/') {
        return Ok((parent, name));
    }
    Ok(("", trimmed))
}

fn encode_graph_path(path: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for segment in path.split('/') {
        if segment.is_empty() {
            continue;
        }
        parts.push(percent_encode(segment));
    }
    parts.join("/")
}

fn percent_encode(input: &str) -> String {
    let mut output = String::new();
    for byte in input.bytes() {
        let unreserved = matches!(
            byte,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~'
        );
        if unreserved {
            output.push(byte as char);
        } else {
            output.push('%');
            output.push_str(&format!("{:02X}", byte));
        }
    }
    output
}

/// SomeDrive's own partial download files (`build_unique_download_temp_path`).
fn is_download_temp_name(name: &str) -> bool {
    name.starts_with(".somedrive-part-") && name.ends_with(".tmp")
}

/// Local files that are never synced: OneNote notebook files (OneNote keeps
/// notebooks in the cloud) and Office's own lock files.
fn is_excluded_local_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".one") || lower.ends_with(".onetoc2") || name.starts_with("~$")
}

fn local_entry_from_metadata(metadata: &std::fs::Metadata) -> LocalEntry {
    let mtime_ns = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_nanos() as i64)
        .unwrap_or(0);
    LocalEntry {
        is_dir: metadata.is_dir(),
        size: if metadata.is_file() { metadata.len() } else { 0 },
        mtime_ns,
    }
}

/// The current local state of `path`, `None` when it does not exist.
fn read_local_entry(path: &Path) -> Result<Option<LocalEntry>, String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() || metadata.is_dir() => {
            Ok(Some(local_entry_from_metadata(&metadata)))
        }
        Ok(_) => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Failed reading '{}': {error}", path.display())),
    }
}

/// Scans the sync folder. Symlinks and special files are skipped, excluded
/// names are left alone, and SomeDrive's own leftover partial downloads are
/// removed. Any unreadable directory fails the scan (a partial scan would
/// look like deletions).
fn collect_local_snapshot_with_progress<F>(
    sync_root: &Path,
    mut on_progress: F,
) -> Result<HashMap<String, LocalEntry>, String>
where
    F: FnMut(&str),
{
    let mut snapshot = HashMap::new();
    let mut stack: Vec<PathBuf> = vec![sync_root.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current).map_err(|error| {
            format!("Failed reading directory '{}': {}", current.display(), error)
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let metadata = entry.metadata().map_err(|error| {
                format!("Failed reading metadata '{}': {}", path.display(), error)
            })?;
            if !metadata.is_file() && !metadata.is_dir() {
                continue;
            }
            if metadata.is_file() && is_download_temp_name(&name) {
                if let Err(error) = std::fs::remove_file(&path) {
                    log::warn!(
                        "LOCAL_TEMP_CLEANUP_FAILED path={} error={}",
                        path.display(),
                        error
                    );
                }
                continue;
            }
            if is_excluded_local_name(&name) {
                continue;
            }

            let relative = path
                .strip_prefix(sync_root)
                .map_err(|error| {
                    format!("Failed computing relative path '{}': {}", path.display(), error)
                })?
                .to_string_lossy()
                .replace('\\', "/");
            let normalized = normalize_relative_path(&relative);
            if normalized.is_empty() {
                continue;
            }
            on_progress(&normalized);
            snapshot.insert(normalized, local_entry_from_metadata(&metadata));
            if metadata.is_dir() {
                stack.push(path);
            }
        }
    }
    Ok(snapshot)
}

const ISSUE_NAME_PREFIX: &str = "name_";
const MAX_ONEDRIVE_PATH_CHARS: usize = 400;
const RESERVED_DEVICE_NAMES: [&str; 4] = ["con", "prn", "aux", "nul"];

/// Why OneDrive cannot hold a file of this name, if it cannot.
fn invalid_onedrive_name_reason(name: &str) -> Option<&'static str> {
    if name.chars().any(|character| "\"*:<>?\\|".contains(character)) {
        return Some("The name contains a character OneDrive does not allow (\" * : < > ? \\ |).");
    }
    if name.chars().any(char::is_control) {
        return Some("The name contains a control character.");
    }
    if name.starts_with(' ') || name.ends_with(' ') || name.ends_with('.') {
        return Some("OneDrive does not allow a name that starts or ends with a space, or ends with a dot.");
    }
    let lower = name.to_lowercase();
    let stem = lower.split('.').next().unwrap_or_default();
    let numbered_device = (stem.starts_with("com") || stem.starts_with("lpt"))
        && stem.len() == 4
        && stem.as_bytes()[3].is_ascii_digit();
    if RESERVED_DEVICE_NAMES.contains(&stem) || numbered_device {
        return Some("The name is reserved by Windows and OneDrive.");
    }
    if lower == ".lock" || lower == "desktop.ini" || lower.contains("_vti_") {
        return Some("OneDrive does not allow this name.");
    }
    None
}

/// Marks local items OneDrive cannot hold as per-file issues: invalid names,
/// over-long paths, names not in Unicode NFC, and names that differ only by
/// case from another item. Items inside an invalid folder share its issue.
/// Recomputed every cycle, so renaming the file clears it.
fn record_local_name_issues(
    profile_id: &str,
    snapshot: &HashMap<String, LocalEntry>,
) -> Result<usize, String> {
    use unicode_normalization::UnicodeNormalization;

    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "UPDATE sync_files SET issue_code = NULL, issue_detail = NULL
             WHERE profile_id = ?1 AND substr(issue_code, 1, length(?2)) = ?2",
            params![profile_id, ISSUE_NAME_PREFIX],
        )
        .map_err(|error| format!("Failed clearing name issues: {error}"))?;

    let mut issues: Vec<(String, &'static str, String)> = Vec::new();
    let mut invalid_folders: Vec<String> = Vec::new();
    let mut paths: Vec<&String> = snapshot.keys().collect();
    paths.sort();
    for path in paths {
        if invalid_folders
            .iter()
            .any(|folder| path.starts_with(&format!("{folder}/")))
        {
            issues.push((
                path.clone(),
                "name_parent_invalid",
                "A folder above this item has a name OneDrive does not allow.".to_string(),
            ));
            continue;
        }
        let name = path.rsplit('/').next().unwrap_or(path);
        let problem = if let Some(reason) = invalid_onedrive_name_reason(name) {
            Some(("name_invalid", reason.to_string()))
        } else if path.chars().count() >= MAX_ONEDRIVE_PATH_CHARS {
            Some((
                "name_path_too_long",
                format!("The path is longer than OneDrive allows ({MAX_ONEDRIVE_PATH_CHARS} characters)."),
            ))
        } else if path.nfc().collect::<String>() != *path {
            Some((
                "name_not_nfc",
                "The name uses decomposed Unicode characters; rename it so OneDrive matches it.".to_string(),
            ))
        } else {
            None
        };
        if let Some((code, detail)) = problem {
            if snapshot.get(path).is_some_and(|entry| entry.is_dir) {
                invalid_folders.push(path.clone());
            }
            issues.push((path.clone(), code, detail));
        }
    }

    // Case collisions: two local names, or a local-only name and a cloud
    // name, that are equal ignoring case. The local-only one is held back.
    let mut known_by_folded: HashMap<String, String> = HashMap::new();
    {
        let mut statement = connection
            .prepare(
                "SELECT path FROM sync_files
                 WHERE profile_id = ?1 AND (cloud_present = 1 OR base_present = 1) AND path != ''",
            )
            .map_err(|error| format!("Failed preparing case collision query: {error}"))?;
        let rows = statement
            .query_map(params![profile_id], |row| row.get::<_, String>(0))
            .map_err(|error| format!("Failed querying tracked paths: {error}"))?;
        for row in rows {
            let path = row.map_err(|error| format!("Failed reading tracked path: {error}"))?;
            known_by_folded.insert(path.to_lowercase(), path);
        }
    }
    let known_paths: HashSet<String> = known_by_folded.values().cloned().collect();
    let mut local_only: Vec<&String> = snapshot
        .keys()
        .filter(|path| !known_paths.contains(*path))
        .collect();
    local_only.sort();
    for path in local_only {
        let folded = path.to_lowercase();
        match known_by_folded.get(&folded) {
            Some(existing) if existing != path => issues.push((
                path.clone(),
                "name_case_collision",
                format!("OneDrive treats this name as the same as '{existing}', which differs only by case."),
            )),
            _ => {
                known_by_folded.insert(folded, path.clone());
            }
        }
    }

    for (path, code, detail) in &issues {
        write_sync_file_issue(&connection, profile_id, path, code, detail)?;
    }
    Ok(issues.len())
}

fn extract_root_relative(parent_path: &str) -> String {
    let mut value = parent_path.trim().to_string();
    if let Some(rest) = value.strip_prefix("/drive/root:") {
        value = rest.to_string();
    }
    value.trim_start_matches('/').to_string()
}

fn normalize_relative_path(value: &str) -> String {
    value
        .replace('\\', "/")
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

fn path_to_local(relative_path: &str) -> PathBuf {
    let mut output = PathBuf::new();
    for segment in relative_path.split('/') {
        if !segment.is_empty() {
            output.push(segment);
        }
    }
    output
}

fn parse_rfc3339_seconds(value: Option<&str>) -> i64 {
    value
        .and_then(|input| chrono::DateTime::parse_from_rfc3339(input).ok())
        .map(|timestamp| timestamp.timestamp())
        .unwrap_or(0)
}

fn shared_drive_id_from_delta_item(item: &GraphDriveItem) -> Option<String> {
    item.remote_item
        .as_ref()
        .and_then(|remote_item| remote_item.get("parentReference"))
        .and_then(|parent_reference| parent_reference.get("driveId"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn shared_item_id_from_delta_item(item: &GraphDriveItem) -> Option<String> {
    item.remote_item
        .as_ref()
        .and_then(|remote_item| remote_item.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn shared_kind_from_delta_item(item: &GraphDriveItem) -> Option<String> {
    let remote_item = item.remote_item.as_ref()?;
    if remote_item.get("folder").is_some() {
        return Some("folder".to_string());
    }
    if remote_item.get("file").is_some() {
        return Some("file".to_string());
    }
    if remote_item.get("package").is_some() {
        return Some("package".to_string());
    }
    Some("unknown".to_string())
}
