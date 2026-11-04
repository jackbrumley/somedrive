use super::*;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAccountSyncRootInput {
    pub id: String,
    pub sync_root: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAccountItemFolderInput {
    pub profile_id: String,
    pub relative_path: String,
}

#[tauri::command]
pub fn open_account_sync_root_folder(
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<(), String> {
    let profile = {
        let _guard = state
            .profiles_lock
            .lock()
            .map_err(|_| "Account profile lock is poisoned".to_string())?;
        let profiles = load_profiles()?;
        profiles
            .into_iter()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| "Account profile not found".to_string())?
    };

    let folder_path = PathBuf::from(profile.sync_root);
    fs::create_dir_all(&folder_path).map_err(|error| {
        format!(
            "Failed to create folder '{}': {}",
            folder_path.display(),
            error
        )
    })?;

    open_folder_in_file_manager(&folder_path)
}

#[tauri::command]
pub fn set_account_sync_root(
    state: tauri::State<'_, AppState>,
    input: SetAccountSyncRootInput,
) -> Result<AccountProfile, String> {
    let path = std::path::PathBuf::from(input.sync_root.trim());
    if !path.is_absolute() {
        return Err("Sync root must be an absolute path".to_string());
    }

    fs::create_dir_all(&path)
        .map_err(|error| format!("Failed to create sync root '{}': {}", path.display(), error))?;

    let _guard = state
        .profiles_lock
        .lock()
        .map_err(|_| "Account profile lock is poisoned".to_string())?;

    let mut profiles = load_profiles()?;
    let profile = profiles
        .iter_mut()
        .find(|profile| profile.id == input.id)
        .ok_or_else(|| "Account profile not found".to_string())?;
    profile.sync_root = path.to_string_lossy().to_string();
    let updated = profile.clone();
    crate::app::account_profiles::save_profiles(&profiles)?;

    let _ = activity_log::append_event(
        &updated.id,
        &updated.email,
        "info",
        &format!(
            "{} Sync root updated",
            log_context::account_prefix_from_parts(&updated.id, &updated.email)
        ),
    );

    Ok(updated)
}

#[tauri::command]
pub fn open_account_item_folder(
    state: tauri::State<'_, AppState>,
    input: OpenAccountItemFolderInput,
) -> Result<(), String> {
    let profile = {
        let _guard = state
            .profiles_lock
            .lock()
            .map_err(|_| "Account profile lock is poisoned".to_string())?;
        let profiles = load_profiles()?;
        profiles
            .into_iter()
            .find(|profile| profile.id == input.profile_id)
            .ok_or_else(|| "Account profile not found".to_string())?
    };

    let relative_path = normalize_relative_sync_item_path(&input.relative_path)?;
    let sync_root = PathBuf::from(profile.sync_root);
    let absolute_path = sync_root.join(relative_path);
    let folder_path = absolute_path.parent().unwrap_or(&sync_root).to_path_buf();

    fs::create_dir_all(&folder_path).map_err(|error| {
        format!(
            "Failed to create folder '{}': {}",
            folder_path.display(),
            error
        )
    })?;

    open_folder_in_file_manager(&folder_path)
}

pub(crate) fn normalize_relative_sync_item_path(value: &str) -> Result<PathBuf, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("Sync path is empty".to_string());
    }
    let input = PathBuf::from(trimmed);
    if input.is_absolute() {
        return Err("Sync path must be relative".to_string());
    }

    let mut normalized = PathBuf::new();
    for component in input.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(segment) => normalized.push(segment),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("Sync path contains invalid segments".to_string())
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err("Sync path is empty".to_string());
    }

    Ok(normalized)
}

pub(crate) fn open_folder_in_file_manager(folder_path: &PathBuf) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(folder_path)
            .spawn()
            .map_err(|error| error.to_string())?;
    }

    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .arg(folder_path)
            .spawn()
            .map_err(|error| error.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(folder_path)
            .spawn()
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}
