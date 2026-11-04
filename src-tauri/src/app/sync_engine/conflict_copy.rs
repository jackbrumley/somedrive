// Conflict copies (docs/sync-engine.md section 6.2): when SomeDrive cannot be
// sure which version to keep, the local version is copied aside as
// `name (conflict YYYY-MM-DD HHMM on <host>).ext` and both are kept. The copy
// is a new local file, so the next cycle uploads it.

fn local_hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|value| value.trim().to_string())
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "this device".to_string())
}

/// A free path next to `local_path` for its conflict copy.
fn conflict_copy_path(local_path: &Path) -> Result<PathBuf, String> {
    let parent = local_path
        .parent()
        .ok_or_else(|| format!("'{}' has no parent folder", local_path.display()))?;
    let file_name = local_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .ok_or_else(|| format!("'{}' has no file name", local_path.display()))?;
    let (stem, extension) = match file_name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem.to_string(), format!(".{extension}")),
        _ => (file_name.clone(), String::new()),
    };
    let stamp = chrono::Local::now().format("%Y-%m-%d %H%M");
    let host = local_hostname();
    for attempt in 1..=100 {
        let suffix = if attempt == 1 {
            String::new()
        } else {
            format!(" {attempt}")
        };
        let candidate = parent.join(format!("{stem} (conflict {stamp} on {host}{suffix}){extension}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(format!("No free conflict copy name next to '{}'", local_path.display()))
}

/// Copies the local file at `local_path` to a conflict copy and checks the
/// copy's size before anything replaces the original.
fn preserve_conflict_copy(local_path: &Path) -> Result<PathBuf, String> {
    let copy_path = conflict_copy_path(local_path)?;
    let copied = std::fs::copy(local_path, &copy_path).map_err(|error| {
        format!(
            "Failed copying '{}' to '{}': {error}",
            local_path.display(),
            copy_path.display()
        )
    })?;
    let original_size = std::fs::metadata(local_path)
        .map_err(|error| format!("Failed reading '{}': {error}", local_path.display()))?
        .len();
    if copied != original_size {
        let _ = std::fs::remove_file(&copy_path);
        return Err(format!(
            "Conflict copy of '{}' is incomplete ({copied} of {original_size} bytes)",
            local_path.display()
        ));
    }
    std::fs::File::open(&copy_path)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("Failed flushing '{}': {error}", copy_path.display()))?;
    Ok(copy_path)
}

#[cfg(test)]
mod conflict_copy_tests {
    use super::*;

    #[test]
    fn conflict_copy_keeps_the_original_and_its_content() {
        let folder = std::env::temp_dir().join(format!("somedrive-conflict-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("create folder");
        let original = folder.join("report.docx");
        std::fs::write(&original, b"local edits").expect("write original");

        let copy = preserve_conflict_copy(&original).expect("preserve copy");

        let copy_name = copy.file_name().unwrap().to_string_lossy().to_string();
        assert!(copy_name.starts_with("report (conflict "), "{copy_name}");
        assert!(copy_name.ends_with(").docx"), "{copy_name}");
        assert_eq!(std::fs::read(&copy).unwrap(), b"local edits");
        assert_eq!(std::fs::read(&original).unwrap(), b"local edits");
        std::fs::remove_dir_all(&folder).expect("clean up");
    }
}
