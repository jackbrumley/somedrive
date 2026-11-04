use super::*;

#[tauri::command]
pub fn pause_all_accounts(state: tauri::State<'_, AppState>) -> Result<u32, String> {
    let _guard = state
        .profiles_lock
        .lock()
        .map_err(|_| "Account profile lock is poisoned".to_string())?;

    let mut profiles = load_profiles()?;
    let mut changed_ids: Vec<String> = Vec::new();
    let mut changed_profiles: Vec<(String, String)> = Vec::new();

    for profile in &mut profiles {
        if profile.agent_state == "syncing" {
            profile.agent_state = "paused".to_string();
            changed_ids.push(profile.id.clone());
            changed_profiles.push((profile.id.clone(), profile.email.clone()));
        }
    }

    if changed_ids.is_empty() {
        return Ok(0);
    }

    save_profiles(&profiles)?;

    for profile_id in &changed_ids {
        sync_engine::on_agent_state_changed(&state, profile_id, "paused")?;
    }

    for (profile_id, profile_email) in &changed_profiles {
        let _ = activity_log::append_event(
            profile_id,
            profile_email,
            "warning",
            &format!(
                "{} Synchronization paused",
                log_context::account_prefix_from_parts(profile_id, profile_email)
            ),
        );
    }

    let _ = activity_log::append_event(
        "global",
        "all-accounts",
        "warning",
        "Paused synchronization for all active accounts",
    );

    Ok(changed_ids.len() as u32)
}

#[tauri::command]
pub fn resume_all_accounts(state: tauri::State<'_, AppState>) -> Result<u32, String> {
    let _guard = state
        .profiles_lock
        .lock()
        .map_err(|_| "Account profile lock is poisoned".to_string())?;

    let mut profiles = load_profiles()?;
    let mut changed_ids: Vec<String> = Vec::new();
    let mut changed_profiles: Vec<(String, String)> = Vec::new();

    for profile in &mut profiles {
        if profile.agent_state == "paused" {
            profile.agent_state = "syncing".to_string();
            changed_ids.push(profile.id.clone());
            changed_profiles.push((profile.id.clone(), profile.email.clone()));
        }
    }

    if changed_ids.is_empty() {
        return Ok(0);
    }

    save_profiles(&profiles)?;

    for profile_id in &changed_ids {
        sync_engine::on_agent_state_changed(&state, profile_id, "syncing")?;
    }

    for (profile_id, profile_email) in &changed_profiles {
        let _ = activity_log::append_event(
            profile_id,
            profile_email,
            "info",
            &format!(
                "{} Synchronization resumed",
                log_context::account_prefix_from_parts(profile_id, profile_email)
            ),
        );
    }

    let _ = activity_log::append_event(
        "global",
        "all-accounts",
        "info",
        "Resumed synchronization for paused accounts",
    );

    Ok(changed_ids.len() as u32)
}
