//! Removing an account: what removal would do to the account's sync folder,
//! and deleting what SomeDrive keeps for the account (its sign-in token and
//! sync database). The command in `commands/accounts/removal_commands.rs`
//! runs the steps in order; this module owns the rules.
//!
//! Files in OneDrive are never touched. The local sync folder is kept unless
//! the user asks to move it to the Trash, and that is refused while any file
//! in it has not reached OneDrive, or when the folder holds more than this
//! account's files (the home folder, SomeDrive's own data, another account).

use crate::app::account_profiles::AccountProfile;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveAccountProfileInput {
    pub id: String,
    /// Also move the account's local sync folder to the Trash.
    pub move_local_folder_to_trash: bool,
}

/// What the removal dialog shows before the user confirms.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountRemovalPreview {
    pub sync_root: String,
    /// Files queued for upload that have not reached OneDrive.
    pub unuploaded_files: usize,
    /// Why the local folder cannot be moved to the Trash; `None` when it can.
    pub local_folder_blocked_reason: Option<String>,
}

/// What removal did. Removal itself succeeded; `warnings` lists the clean-up
/// steps after it that failed, for the user to finish by hand.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountRemovalOutcome {
    pub local_folder_trashed: bool,
    pub warnings: Vec<String>,
}

/// The preview for `profile`, given the other accounts and its count of
/// files not yet uploaded.
pub fn removal_preview(
    profile: &AccountProfile,
    all_profiles: &[AccountProfile],
    unuploaded_files: usize,
) -> Result<AccountRemovalPreview, String> {
    let sync_root = PathBuf::from(&profile.sync_root);
    let blocked_reason = if !sync_root.is_dir() {
        Some("The sync folder no longer exists.".to_string())
    } else {
        let other_accounts: Vec<(String, PathBuf)> = all_profiles
            .iter()
            .filter(|other| other.id != profile.id)
            .map(|other| {
                (
                    other.display_name.clone(),
                    resolve(Path::new(&other.sync_root)),
                )
            })
            .collect();
        local_folder_blocked_reason(
            &resolve(&sync_root),
            &resolve(&crate::paths::home_dir()?),
            &resolve(&crate::paths::config_root()?),
            &other_accounts,
            unuploaded_files,
        )
    };
    Ok(AccountRemovalPreview {
        sync_root: profile.sync_root.clone(),
        unuploaded_files,
        local_folder_blocked_reason: blocked_reason,
    })
}

/// The real path when the folder exists (symlinks resolved), so two spellings
/// of one folder compare equal.
fn resolve(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Why `sync_root` must not go to the Trash, or `None` when it may. All paths
/// are resolved.
fn local_folder_blocked_reason(
    sync_root: &Path,
    home: &Path,
    config_root: &Path,
    other_accounts: &[(String, PathBuf)],
    unuploaded_files: usize,
) -> Option<String> {
    if home.starts_with(sync_root) {
        return Some("The sync folder is or contains your home folder.".to_string());
    }
    if config_root.starts_with(sync_root) {
        return Some("The sync folder contains SomeDrive's own settings.".to_string());
    }
    if let Some((name, _)) = other_accounts.iter().find(|(_, other_root)| {
        other_root.starts_with(sync_root) || sync_root.starts_with(other_root)
    }) {
        return Some(format!(
            "The sync folder overlaps the sync folder of {name}."
        ));
    }
    if unuploaded_files > 0 {
        return Some(format!(
            "{} {} not reached OneDrive yet. Let sync finish first, or keep the folder.",
            unuploaded_files,
            if unuploaded_files == 1 {
                "file has"
            } else {
                "files have"
            }
        ));
    }
    None
}

/// Deletes the account's folder under `~/.config/somedrive/accounts`: its
/// sign-in token (`auth.json`) and sync database.
pub fn delete_account_data(profile_id: &str) -> Result<(), String> {
    let account_dir = crate::paths::account_dir(profile_id)?;
    match fs::remove_dir_all(&account_dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Could not delete the account's data in {}: {error}",
            account_dir.display()
        )),
    }
}

/// Moves the account's sync folder to the Trash.
pub fn move_sync_root_to_trash(sync_root: &str) -> Result<(), String> {
    trash::delete(sync_root)
        .map_err(|error| format!("Could not move {sync_root} to the Trash: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocked(
        sync_root: &str,
        others: &[(&str, &str)],
        unuploaded_files: usize,
    ) -> Option<String> {
        let other_accounts: Vec<(String, PathBuf)> = others
            .iter()
            .map(|(name, root)| (name.to_string(), PathBuf::from(root)))
            .collect();
        local_folder_blocked_reason(
            Path::new(sync_root),
            Path::new("/home/user"),
            Path::new("/home/user/.config/somedrive"),
            &other_accounts,
            unuploaded_files,
        )
    }

    #[test]
    fn an_own_folder_with_everything_uploaded_may_go_to_the_trash() {
        assert_eq!(
            blocked(
                "/home/user/SomeDrive/work",
                &[("Home", "/home/user/SomeDrive/home")],
                0
            ),
            None
        );
    }

    #[test]
    fn the_home_folder_or_a_folder_above_it_is_refused() {
        assert!(blocked("/home/user", &[], 0).is_some());
        assert!(blocked("/home", &[], 0).is_some());
        assert!(blocked("/", &[], 0).is_some());
    }

    #[test]
    fn a_folder_holding_somedrive_settings_is_refused() {
        assert!(blocked("/home/user/.config", &[], 0).is_some());
    }

    #[test]
    fn a_folder_shared_with_another_account_is_refused() {
        let reason = blocked(
            "/home/user/SomeDrive",
            &[("Home", "/home/user/SomeDrive/home")],
            0,
        );
        assert_eq!(
            reason.as_deref(),
            Some("The sync folder overlaps the sync folder of Home.")
        );
        assert!(blocked(
            "/home/user/SomeDrive/home/inner",
            &[("Home", "/home/user/SomeDrive/home")],
            0
        )
        .is_some());
        assert!(blocked(
            "/home/user/SomeDrive/home",
            &[("Home", "/home/user/SomeDrive/home")],
            0
        )
        .is_some());
    }

    #[test]
    fn a_sibling_with_a_longer_name_is_not_an_overlap() {
        assert_eq!(
            blocked(
                "/home/user/SomeDrive/home2",
                &[("Home", "/home/user/SomeDrive/home")],
                0
            ),
            None
        );
    }

    #[test]
    fn files_not_yet_uploaded_block_the_trash() {
        assert_eq!(
            blocked("/home/user/SomeDrive/work", &[], 3).as_deref(),
            Some(
                "3 files have not reached OneDrive yet. Let sync finish first, or keep the folder."
            )
        );
        assert!(blocked("/home/user/SomeDrive/work", &[], 1)
            .unwrap()
            .starts_with("1 file has"));
    }
}
