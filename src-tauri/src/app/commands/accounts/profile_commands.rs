use super::*;

#[tauri::command]
pub fn list_account_profiles(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AccountProfile>, String> {
    let _guard = state
        .profiles_lock
        .lock()
        .map_err(|_| "Account profile lock is poisoned".to_string())?;
    load_profiles()
}

#[tauri::command]
pub fn create_account_profile(
    state: tauri::State<'_, AppState>,
    input: CreateAccountProfileInput,
) -> Result<AccountProfile, String> {
    let _guard = state
        .profiles_lock
        .lock()
        .map_err(|_| "Account profile lock is poisoned".to_string())?;
    let profile = create_profile(input)?;
    sync_engine::runtime_set_profile_auth_ready(&state.sync_runtime, &profile.id, false);
    let _ = activity_log::append_event(
        &profile.id,
        &profile.email,
        "success",
        &format!(
            "{} Account profile created",
            log_context::account_prefix_from_parts(&profile.id, &profile.email)
        ),
    );
    Ok(profile)
}

#[tauri::command]
pub fn rename_account_profile(
    state: tauri::State<'_, AppState>,
    input: RenameAccountProfileInput,
) -> Result<AccountProfile, String> {
    let _guard = state
        .profiles_lock
        .lock()
        .map_err(|_| "Account profile lock is poisoned".to_string())?;
    let profile = rename_profile(input)?;
    let _ = activity_log::append_event(
        &profile.id,
        &profile.email,
        "info",
        &format!(
            "{} Account profile renamed",
            log_context::account_prefix_from_parts(&profile.id, &profile.email)
        ),
    );
    Ok(profile)
}

#[tauri::command]
pub fn remove_account_profile(
    state: tauri::State<'_, AppState>,
    input: RemoveAccountProfileInput,
) -> Result<(), String> {
    let _guard = state
        .profiles_lock
        .lock()
        .map_err(|_| "Account profile lock is poisoned".to_string())?;
    let profile_id = input.id.clone();
    remove_profile(input)?;
    sync_engine::on_agent_state_changed(&state, &profile_id, "idle")?;
    if let Ok(mut runtime_map) = state.sync_runtime.lock() {
        sync_runtime::remove_account(&mut runtime_map, &profile_id);
    }
    let _ = activity_log::append_event(
        &profile_id,
        &profile_id,
        "warning",
        &format!(
            "{} Account profile removed",
            log_context::account_prefix(&profile_id)
        ),
    );
    Ok(())
}

#[tauri::command]
pub fn set_account_agent_state(
    state: tauri::State<'_, AppState>,
    input: SetAccountAgentStateInput,
) -> Result<AccountProfile, String> {
    let _guard = state
        .profiles_lock
        .lock()
        .map_err(|_| "Account profile lock is poisoned".to_string())?;
    let profile = set_agent_state(input)?;
    log::info!(
        "{} SYNC_AGENT_STATE_SET state={}",
        log_context::account_prefix_from_parts(&profile.id, &profile.email),
        profile.agent_state
    );
    sync_engine::on_agent_state_changed(&state, &profile.id, &profile.agent_state)?;
    let message = format!(
        "{} Agent state changed to {}",
        log_context::account_prefix_from_parts(&profile.id, &profile.email),
        profile.agent_state
    );
    let _ = activity_log::append_event(&profile.id, &profile.email, "info", &message);
    Ok(profile)
}

#[tauri::command]
pub fn confirm_account_large_delete(
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<(), String> {
    sync_engine::confirm_large_delete_guard(&state, &profile_id)?;
    let _ = activity_log::append_event(
        &profile_id,
        &log_context::account_identity(&profile_id),
        "warning",
        &format!(
            "{} Large deletion confirmed by user",
            log_context::account_prefix(&profile_id)
        ),
    );
    Ok(())
}

#[tauri::command]
pub fn keep_cloud_files_after_large_delete(
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<(), String> {
    sync_engine::keep_cloud_files_after_large_delete(&state, &profile_id)?;
    let _ = activity_log::append_event(
        &profile_id,
        &log_context::account_identity(&profile_id),
        "info",
        &format!(
            "{} Kept cloud files after large deletion warning",
            log_context::account_prefix(&profile_id)
        ),
    );
    Ok(())
}

#[tauri::command]
pub fn retry_failed_download(
    profile_id: String,
    recent_item_id: String,
) -> Result<RetryFailedDownloadResponse, String> {
    let status = sync_engine::retry_failed_download_job(&profile_id, &recent_item_id)?;
    let status_text = match status {
        sync_engine::RetryFailedDownloadJobStatus::Retried => "retried",
        sync_engine::RetryFailedDownloadJobStatus::AlreadyRetrying => "already_retrying",
    };
    let _ = activity_log::append_event(
        &profile_id,
        &log_context::account_identity(&profile_id),
        "info",
        &format!(
            "{} Retry failed download item {} status={}",
            log_context::account_prefix(&profile_id),
            recent_item_id,
            status_text
        ),
    );
    Ok(RetryFailedDownloadResponse {
        status: status_text.to_string(),
    })
}

#[tauri::command]
pub fn retry_all_failed_downloads(
    profile_id: String,
) -> Result<RetryAllFailedDownloadsResponse, String> {
    let report = sync_engine::retry_all_failed_download_jobs(&profile_id)?;
    let _ = activity_log::append_event(
        &profile_id,
        &log_context::account_identity(&profile_id),
        "info",
        &format!(
            "{} Retry-all failed downloads retried={} skipped_permission_denied={} already_retrying={}",
            log_context::account_prefix(&profile_id),
            report.retried,
            report.skipped_permission_denied,
            report.already_retrying
        ),
    );
    Ok(RetryAllFailedDownloadsResponse {
        retried: report.retried,
        skipped_permission_denied: report.skipped_permission_denied,
        already_retrying: report.already_retrying,
    })
}

#[tauri::command]
pub fn get_account_large_delete_preview(profile_id: String) -> Result<Vec<String>, String> {
    sync_engine::get_large_delete_pending_paths(&profile_id)
}

#[tauri::command]
pub fn export_account_large_delete_preview(
    profile_id: String,
    destination_path: String,
) -> Result<String, String> {
    let pending_paths = sync_engine::get_large_delete_pending_paths(&profile_id)?;
    if pending_paths.is_empty() {
        return Err("No pending large deletion paths to export".to_string());
    }

    let mut output = String::new();
    output.push_str("SomeDrive Large Deletion Review\n");
    output.push_str(&format!("Profile: {}\n", profile_id));
    output.push_str(&format!(
        "Generated: {}\n",
        chrono::Local::now().to_rfc3339()
    ));
    output.push_str(&format!("Items: {}\n\n", pending_paths.len()));
    for path in pending_paths {
        output.push_str(&path);
        output.push('\n');
    }

    let destination = std::path::PathBuf::from(destination_path.clone());
    if let Some(parent) = destination.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Failed creating export directory: {}", error))?;
        }
    }
    fs::write(&destination, output).map_err(|error| {
        format!(
            "Failed writing large deletion export '{}': {}",
            destination.display(),
            error
        )
    })?;

    Ok(destination_path)
}
#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct RetryFailedDownloadResponse {
    pub status: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryAllFailedDownloadsResponse {
    pub retried: usize,
    pub skipped_permission_denied: usize,
    pub already_retrying: usize,
}
