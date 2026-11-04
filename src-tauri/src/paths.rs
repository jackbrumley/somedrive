//! Single owner of SomeDrive's on-disk locations (App Standard section 3).
//! Nothing else calls `dirs::` directly.
//!
//! Everything SomeDrive keeps lives under `~/.config/somedrive`
//! (`$XDG_CONFIG_HOME/somedrive`): settings, accounts, sign-in tokens, sync
//! databases, activity and debug logs. Synced files live in the user's sync
//! roots, by default `~/SomeDrive/<account>`.

use std::fs;
use std::path::PathBuf;

const APP_DIR_NAME: &str = "somedrive";

/// `~/.config/somedrive`, not created.
#[cfg(not(test))]
pub fn config_root() -> Result<PathBuf, String> {
    let base = dirs::config_dir().ok_or("Could not resolve the config directory")?;
    Ok(base.join(APP_DIR_NAME))
}

/// Tests never touch the real `~/.config/somedrive`: each test process gets
/// its own root under the system temp directory.
#[cfg(test)]
pub fn config_root() -> Result<PathBuf, String> {
    Ok(test_root().join("config").join(APP_DIR_NAME))
}

#[cfg(test)]
fn test_root() -> PathBuf {
    std::env::temp_dir().join(format!("somedrive-test-{}", std::process::id()))
}

fn ensure_dir(dir: PathBuf) -> Result<PathBuf, String> {
    fs::create_dir_all(&dir)
        .map_err(|error| format!("Failed creating directory '{}': {error}", dir.display()))?;
    Ok(dir)
}

/// `~/.config/somedrive/app_settings.json`.
pub fn app_settings_file() -> Result<PathBuf, String> {
    Ok(ensure_dir(config_root()?)?.join("app_settings.json"))
}

/// `~/.config/somedrive/accounts`: `profiles.json` plus one folder per account.
pub fn accounts_dir() -> Result<PathBuf, String> {
    Ok(config_root()?.join("accounts"))
}

/// `~/.config/somedrive/accounts/<profile_id>`: the account's `auth.json` and
/// `sync_jobs.db`.
pub fn account_dir(profile_id: &str) -> Result<PathBuf, String> {
    Ok(accounts_dir()?.join(profile_id))
}

/// `~/.config/somedrive/activity`.
pub fn activity_dir() -> Result<PathBuf, String> {
    Ok(config_root()?.join("activity"))
}

/// `~/.config/somedrive/debug`, created: session, profile, raw and update logs.
pub fn debug_dir() -> Result<PathBuf, String> {
    ensure_dir(config_root()?.join("debug"))
}

/// Where new accounts sync by default: `~/SomeDrive`.
#[cfg(not(test))]
pub fn default_sync_roots_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("Could not resolve the home directory")?;
    Ok(home.join("SomeDrive"))
}

#[cfg(test)]
pub fn default_sync_roots_dir() -> Result<PathBuf, String> {
    Ok(test_root().join("SomeDrive"))
}
