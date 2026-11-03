use crate::app::account_profiles::load_profiles;
use crate::app::state::AppState;
use crate::app::sync_engine::build_authoritative_runtime_status;
use crate::app::sync_runtime::{
    emit_sync_status_snapshot_accounts, recompute_authority_fields, snapshot, SyncRuntimeSnapshot,
};

fn build_authoritative_sync_runtime_snapshot(
    state: &tauri::State<'_, AppState>,
) -> Result<SyncRuntimeSnapshot, String> {
    let runtime = state
        .sync_runtime
        .lock()
        .map_err(|_| "Sync runtime lock is poisoned".to_string())?;
    let mut runtime_snapshot = snapshot(&runtime);
    drop(runtime);

    let profile_auth_by_id: std::collections::HashMap<String, bool> = load_profiles()?
        .into_iter()
        .map(|profile| (profile.id, profile.auth_configured))
        .collect();

    let runtime_auth_by_id: std::collections::HashMap<String, bool> = runtime_snapshot
        .accounts
        .iter()
        .map(|account| (account.profile_id.clone(), account.auth_ready))
        .collect();

    let mut authoritative_accounts = Vec::with_capacity(runtime_snapshot.accounts.len());
    for account in &runtime_snapshot.accounts {
        let auth_ready = profile_auth_by_id
            .get(&account.profile_id)
            .copied()
            .unwrap_or_else(|| {
                runtime_auth_by_id
                    .get(&account.profile_id)
                    .copied()
                    .unwrap_or(false)
            });
        let authoritative = build_authoritative_runtime_status(&account.profile_id, auth_ready)?;
        authoritative_accounts.push(authoritative);
    }

    runtime_snapshot.accounts = authoritative_accounts;

    for account in &mut runtime_snapshot.accounts {
        recompute_authority_fields(account);
    }

    runtime_snapshot.revision = runtime_snapshot
        .revision
        .max(runtime_revision_from_updated_at(&runtime_snapshot));

    Ok(runtime_snapshot)
}

fn runtime_revision_from_updated_at(snapshot: &SyncRuntimeSnapshot) -> u64 {
    snapshot
        .accounts
        .iter()
        .filter_map(|account| chrono::DateTime::parse_from_rfc3339(&account.updated_at).ok())
        .map(|value| value.timestamp_millis().max(0) as u64)
        .max()
        .unwrap_or(0)
}

#[tauri::command]
pub fn get_sync_runtime_snapshot(
    state: tauri::State<'_, AppState>,
) -> Result<SyncRuntimeSnapshot, String> {
    build_authoritative_sync_runtime_snapshot(&state)
}

#[tauri::command]
pub fn request_sync_status_snapshot(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let runtime_snapshot = build_authoritative_sync_runtime_snapshot(&state)?;
    emit_sync_status_snapshot_accounts(&runtime_snapshot.accounts);
    Ok(())
}
