use super::*;
use crate::app::account_removal::{
    delete_account_data, move_sync_root_to_trash, removal_preview, AccountRemovalOutcome,
    AccountRemovalPreview, RemoveAccountProfileInput,
};

fn load_profiles_locked(state: &tauri::State<'_, AppState>) -> Result<Vec<AccountProfile>, String> {
    let _guard = state
        .profiles_lock
        .lock()
        .map_err(|_| "Account profile lock is poisoned".to_string())?;
    load_profiles()
}

fn find_profile(profiles: &[AccountProfile], profile_id: &str) -> Result<AccountProfile, String> {
    profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .cloned()
        .ok_or_else(|| "Account profile not found".to_string())
}

fn preview_for(
    state: &tauri::State<'_, AppState>,
    profile_id: &str,
) -> Result<(AccountProfile, AccountRemovalPreview), String> {
    let profiles = load_profiles_locked(state)?;
    let profile = find_profile(&profiles, profile_id)?;
    let unuploaded_files = sync_engine::count_unuploaded_files(profile_id)?;
    let preview = removal_preview(&profile, &profiles, unuploaded_files)?;
    Ok((profile, preview))
}

/// What the removal dialog shows: the sync folder, and whether it may go to
/// the Trash.
#[tauri::command]
pub fn get_account_removal_preview(
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<AccountRemovalPreview, String> {
    Ok(preview_for(&state, &profile_id)?.1)
}

/// Removes an account: stops its sync, forgets it, deletes its sign-in token
/// and sync database, and (if asked) moves its sync folder to the Trash.
///
/// Fails with the reason, having removed nothing, until the account is gone
/// from the account list; sync then resumes as it was. Clean-up steps after
/// that which fail are returned as warnings.
#[tauri::command]
pub async fn remove_account_profile(
    state: tauri::State<'_, AppState>,
    input: RemoveAccountProfileInput,
) -> Result<AccountRemovalOutcome, String> {
    if !sync_engine::sync_accepts_new_work() {
        return Err(
            "SomeDrive is shutting down. Remove the account after it starts again.".to_string(),
        );
    }
    let (profile, preview) = preview_for(&state, &input.id)?;
    let account_prefix = log_context::account_prefix_from_parts(&profile.id, &profile.email);
    log::info!(
        "{account_prefix} ACCOUNT_REMOVAL_BEGIN move_local_folder_to_trash={}",
        input.move_local_folder_to_trash
    );
    if input.move_local_folder_to_trash {
        if let Some(reason) = preview.local_folder_blocked_reason {
            log::info!("{account_prefix} ACCOUNT_REMOVAL_REFUSED reason={reason}");
            return Err(reason);
        }
    }

    if let Err(error) = sync_engine::stop_account_sync(&state, &profile.id).await {
        return Err(abandon_removal(&state, &profile, &account_prefix, error));
    }
    if input.move_local_folder_to_trash {
        // Checked again with sync stopped: an upload may have been queued
        // while the dialog was open.
        match preview_for(&state, &profile.id) {
            Ok((
                _,
                AccountRemovalPreview {
                    local_folder_blocked_reason: None,
                    ..
                },
            )) => {}
            Ok((
                _,
                AccountRemovalPreview {
                    local_folder_blocked_reason: Some(reason),
                    ..
                },
            ))
            | Err(reason) => {
                return Err(abandon_removal(&state, &profile, &account_prefix, reason))
            }
        }
    }

    {
        let _guard = state
            .profiles_lock
            .lock()
            .map_err(|_| "Account profile lock is poisoned".to_string())?;
        if let Err(error) = remove_profile(&profile.id) {
            drop(_guard);
            return Err(abandon_removal(&state, &profile, &account_prefix, error));
        }
    }
    log::info!("{account_prefix} ACCOUNT_REMOVED_FROM_LIST");

    let mut warnings = Vec::new();
    if let Err(error) = sync_engine::forget_removed_account(&state, &profile.id) {
        log::error!("{account_prefix} ACCOUNT_REMOVAL_FORGET_FAILED error={error}");
        warnings.push(error);
    }
    match delete_account_data(&profile.id) {
        Ok(()) => log::info!("{account_prefix} ACCOUNT_DATA_DELETED"),
        Err(error) => {
            log::error!("{account_prefix} ACCOUNT_DATA_DELETE_FAILED error={error}");
            warnings.push(error);
        }
    }
    let mut local_folder_trashed = false;
    if input.move_local_folder_to_trash {
        match move_sync_root_to_trash(&profile.sync_root) {
            Ok(()) => {
                local_folder_trashed = true;
                log::info!(
                    "{account_prefix} ACCOUNT_SYNC_FOLDER_TRASHED path={}",
                    profile.sync_root
                );
            }
            Err(error) => {
                log::error!("{account_prefix} ACCOUNT_SYNC_FOLDER_TRASH_FAILED error={error}");
                warnings.push(error);
            }
        }
    }

    let _ = activity_log::append_event(
        &profile.id,
        &profile.email,
        "warning",
        &format!(
            "{account_prefix} Account removed{}",
            if local_folder_trashed {
                "; sync folder moved to the Trash"
            } else {
                "; sync folder kept"
            }
        ),
    );
    log::info!(
        "{account_prefix} ACCOUNT_REMOVAL_COMPLETE local_folder_trashed={local_folder_trashed} warnings={}",
        warnings.len()
    );
    Ok(AccountRemovalOutcome {
        local_folder_trashed,
        warnings,
    })
}

/// Puts the account back as it was (syncing again if it was syncing) and
/// returns the reason removal stopped.
fn abandon_removal(
    state: &tauri::State<'_, AppState>,
    profile: &AccountProfile,
    account_prefix: &str,
    reason: String,
) -> String {
    log::warn!("{account_prefix} ACCOUNT_REMOVAL_ABANDONED reason={reason}");
    if let Err(error) =
        sync_engine::on_agent_state_changed(state, &profile.id, &profile.agent_state)
    {
        log::error!("{account_prefix} ACCOUNT_REMOVAL_RESUME_FAILED error={error}");
        return format!("{reason} Sync could not restart either: {error}");
    }
    reason
}
