use crate::app::json_store;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::app::sync_engine::{
    ensure_sync_lifecycle_row, persist_sync_lifecycle_agent_state,
    persist_sync_lifecycle_last_sync_at, read_sync_lifecycle_profile_metadata,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountProfile {
    pub id: String,
    pub display_name: String,
    pub email: String,
    pub slug: String,
    pub kind: String,
    pub sync_root: String,
    pub auth_configured: bool,
    pub agent_state: String,
    pub last_sync_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAccountProfileInput {
    pub display_name: String,
    pub kind: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameAccountProfileInput {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveAccountProfileInput {
    pub id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAccountAgentStateInput {
    pub id: String,
    pub agent_state: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct AccountProfileStore {
    profiles: Vec<AccountProfile>,
}

pub fn load_profiles() -> Result<Vec<AccountProfile>, String> {
    let Some(mut store) = json_store::read_json::<AccountProfileStore>(&storage_file_path()?)?
    else {
        return Ok(Vec::new());
    };
    for profile in &mut store.profiles {
        match read_sync_lifecycle_profile_metadata(&profile.id) {
            Ok(Some((agent_state, last_sync_at))) => {
                profile.agent_state = agent_state;
                profile.last_sync_at = last_sync_at;
            }
            // A new or rebuilt sync database: recreate the account's
            // lifecycle row from its saved profile.
            Ok(None) => match ensure_sync_lifecycle_row(
                &profile.id,
                &profile.agent_state,
                profile.last_sync_at.as_deref(),
            ) {
                Ok(true) => log::info!(
                    "[acct:{}] SYNC_LIFECYCLE_ROW_CREATED agent_state={} reason=missing_from_database",
                    crate::app::log_context::account_identity_from_parts(&profile.id, &profile.email),
                    profile.agent_state
                ),
                Ok(false) => {}
                Err(error) => log::warn!(
                    "[acct:{}] SYNC_LIFECYCLE_ROW_CREATE_FAILED error={}",
                    crate::app::log_context::account_identity_from_parts(&profile.id, &profile.email),
                    error
                ),
            },
            Err(error) => {
                log::warn!(
                    "{} PROFILE_METADATA_DB_HYDRATE_FAILED error={}",
                    crate::app::log_context::account_prefix(&profile.id),
                    error
                );
            }
        }
    }
    Ok(store.profiles)
}

pub fn save_profiles(profiles: &[AccountProfile]) -> Result<(), String> {
    let payload = AccountProfileStore {
        profiles: profiles.to_vec(),
    };
    json_store::write_json(&storage_file_path()?, &payload)
}

pub fn create_profile(input: CreateAccountProfileInput) -> Result<AccountProfile, String> {
    let display_name = input.display_name.trim();
    if display_name.is_empty() {
        return Err("Display name is required".to_string());
    }

    let kind = normalize_kind(&input.kind)?;
    let mut profiles = load_profiles()?;
    let unique_slug = build_unique_slug(display_name, &profiles);
    let sync_root = default_sync_root(&unique_slug)?;

    fs::create_dir_all(&sync_root).map_err(|error| {
        format!(
            "Failed to create sync folder '{}': {}",
            sync_root.display(),
            error
        )
    })?;

    let profile = AccountProfile {
        id: generate_profile_id(),
        display_name: display_name.to_string(),
        email: String::new(),
        slug: unique_slug,
        kind,
        sync_root: sync_root.to_string_lossy().to_string(),
        auth_configured: false,
        agent_state: "idle".to_string(),
        last_sync_at: None,
    };

    profiles.push(profile.clone());
    save_profiles(&profiles)?;
    persist_sync_lifecycle_agent_state(&profile.id, &profile.agent_state)?;
    persist_sync_lifecycle_last_sync_at(&profile.id, None)?;
    Ok(profile)
}

pub fn rename_profile(input: RenameAccountProfileInput) -> Result<AccountProfile, String> {
    let display_name = input.display_name.trim();
    if display_name.is_empty() {
        return Err("Display name is required".to_string());
    }

    let mut profiles = load_profiles()?;
    let index = profiles
        .iter()
        .position(|profile| profile.id == input.id)
        .ok_or_else(|| "Account profile not found".to_string())?;

    profiles[index].display_name = display_name.to_string();
    let updated = profiles[index].clone();
    save_profiles(&profiles)?;
    Ok(updated)
}

pub fn remove_profile(input: RemoveAccountProfileInput) -> Result<(), String> {
    let mut profiles = load_profiles()?;
    let initial_len = profiles.len();
    profiles.retain(|profile| profile.id != input.id);
    if profiles.len() == initial_len {
        return Err("Account profile not found".to_string());
    }
    save_profiles(&profiles)
}

pub fn set_agent_state(input: SetAccountAgentStateInput) -> Result<AccountProfile, String> {
    let next_state = normalize_agent_state(&input.agent_state)?;
    let mut profiles = load_profiles()?;
    let index = profiles
        .iter()
        .position(|profile| profile.id == input.id)
        .ok_or_else(|| "Account profile not found".to_string())?;

    profiles[index].agent_state = next_state;
    let updated = profiles[index].clone();
    save_profiles(&profiles)?;
    persist_sync_lifecycle_agent_state(&updated.id, &updated.agent_state)?;
    Ok(updated)
}

fn storage_dir_path() -> Result<PathBuf, String> {
    crate::paths::accounts_dir()
}

pub fn storage_file_path() -> Result<PathBuf, String> {
    Ok(storage_dir_path()?.join("profiles.json"))
}

fn default_sync_root(slug: &str) -> Result<PathBuf, String> {
    Ok(crate::paths::default_sync_roots_dir()?.join(slug))
}

fn build_unique_slug(display_name: &str, profiles: &[AccountProfile]) -> String {
    let base = slugify(display_name);
    if !profiles.iter().any(|profile| profile.slug == base) {
        return base;
    }

    let mut suffix = 2_u32;
    loop {
        let candidate = format!("{}-{}", base, suffix);
        if !profiles.iter().any(|profile| profile.slug == candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

fn slugify(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut prev_hyphen = false;

    for character in input.chars() {
        if character.is_ascii_alphanumeric() {
            output.push(character.to_ascii_lowercase());
            prev_hyphen = false;
        } else if !prev_hyphen {
            output.push('-');
            prev_hyphen = true;
        }
    }

    let trimmed = output.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "account".to_string()
    } else {
        trimmed
    }
}

fn normalize_kind(value: &str) -> Result<String, String> {
    match value.trim().to_lowercase().as_str() {
        "personal" => Ok("personal".to_string()),
        "business" => Ok("business".to_string()),
        _ => Err("Account kind must be 'personal' or 'business'".to_string()),
    }
}

fn normalize_agent_state(value: &str) -> Result<String, String> {
    match value.trim().to_lowercase().as_str() {
        "idle" => Ok("idle".to_string()),
        "syncing" => Ok("syncing".to_string()),
        "paused" => Ok("paused".to_string()),
        "error" => Ok("error".to_string()),
        _ => Err("Agent state must be one of: idle, syncing, paused, error".to_string()),
    }
}

fn generate_profile_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    format!("profile-{}-{}", nanos, pid)
}
