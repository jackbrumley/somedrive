//! Single owner of SomeDrive's on-disk locations (App Standard section 3).
//! Nothing else calls `dirs::` directly.
//!
//! Everything SomeDrive keeps lives under `~/.config/somedrive`
//! (`$XDG_CONFIG_HOME/somedrive`): settings, accounts, sign-in tokens, sync
//! databases, activity and debug logs. Synced files live in the user's sync
//! roots, by default `~/SomeDrive/<account>`.

use std::fs;
use std::path::{Path, PathBuf};

const APP_DIR_NAME: &str = "somedrive";
const SETTINGS_FILE_NAME: &str = "settings.json";
/// The settings file's name before every app standardised on `settings.json`.
const LEGACY_SETTINGS_FILE_NAME: &str = "app_settings.json";

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

/// `~/.config/somedrive/settings.json`.
pub fn app_settings_file() -> Result<PathBuf, String> {
    Ok(ensure_dir(config_root()?)?.join(SETTINGS_FILE_NAME))
}

/// What the one-time settings file rename did, for the session log.
#[derive(Debug, PartialEq, Eq)]
pub enum SettingsFileMigration {
    /// `app_settings.json` was renamed to `settings.json`.
    Renamed(String),
    /// Both files exist: `settings.json` is kept, the legacy file is left
    /// untouched for the user to inspect.
    KeptNew(String),
    /// The rename failed; the legacy file is left untouched and start-up
    /// stops, so the settings are never replaced by defaults.
    Failed(String),
}

/// One-time migration of `~/.config/somedrive/app_settings.json` to
/// `settings.json`. Must run before anything reads the settings, or the
/// settings load would create a default `settings.json` first.
///
/// Returns `None` when there was nothing to migrate. Every other outcome is
/// explicit, and the legacy file is only ever moved by a successful rename.
pub fn migrate_legacy_settings_file() -> Option<SettingsFileMigration> {
    match config_root() {
        Ok(root) => migrate_legacy_settings_file_in(&root),
        Err(error) => {
            Some(SettingsFileMigration::Failed(format!(
                "Could not resolve the config directory to check for {LEGACY_SETTINGS_FILE_NAME}: {error}"
            )))
        }
    }
}

fn migrate_legacy_settings_file_in(config_dir: &Path) -> Option<SettingsFileMigration> {
    let legacy_file = config_dir.join(LEGACY_SETTINGS_FILE_NAME);
    if !legacy_file.exists() {
        return None;
    }
    let settings_file = config_dir.join(SETTINGS_FILE_NAME);
    if settings_file.exists() {
        return Some(SettingsFileMigration::KeptNew(format!(
            "both {} and {} exist; keeping {}, legacy file left untouched",
            legacy_file.display(),
            settings_file.display(),
            settings_file.display()
        )));
    }
    match fs::rename(&legacy_file, &settings_file) {
        Ok(()) => Some(SettingsFileMigration::Renamed(format!(
            "renamed {} to {}",
            legacy_file.display(),
            settings_file.display()
        ))),
        Err(error) => Some(SettingsFileMigration::Failed(format!(
            "Could not rename the settings file {} to {}: {error}",
            legacy_file.display(),
            settings_file.display()
        ))),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config_dir(name: &str) -> PathBuf {
        let directory = test_root().join(format!("settings-migration-{name}"));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn legacy_settings_file_is_renamed() {
        let directory = temp_config_dir("rename");
        fs::write(directory.join(LEGACY_SETTINGS_FILE_NAME), b"legacy").unwrap();

        let outcome = migrate_legacy_settings_file_in(&directory);

        assert!(matches!(outcome, Some(SettingsFileMigration::Renamed(_))));
        assert!(!directory.join(LEGACY_SETTINGS_FILE_NAME).exists());
        assert_eq!(
            fs::read(directory.join(SETTINGS_FILE_NAME)).unwrap(),
            b"legacy"
        );
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn existing_settings_file_is_kept_when_both_exist() {
        let directory = temp_config_dir("both");
        fs::write(directory.join(LEGACY_SETTINGS_FILE_NAME), b"legacy").unwrap();
        fs::write(directory.join(SETTINGS_FILE_NAME), b"current").unwrap();

        let outcome = migrate_legacy_settings_file_in(&directory);

        assert!(matches!(outcome, Some(SettingsFileMigration::KeptNew(_))));
        assert_eq!(
            fs::read(directory.join(SETTINGS_FILE_NAME)).unwrap(),
            b"current"
        );
        assert_eq!(
            fs::read(directory.join(LEGACY_SETTINGS_FILE_NAME)).unwrap(),
            b"legacy"
        );
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn nothing_happens_without_a_legacy_file() {
        let directory = temp_config_dir("none");
        fs::write(directory.join(SETTINGS_FILE_NAME), b"current").unwrap();

        assert_eq!(migrate_legacy_settings_file_in(&directory), None);
        assert_eq!(
            fs::read(directory.join(SETTINGS_FILE_NAME)).unwrap(),
            b"current"
        );
        fs::remove_dir_all(&directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn failed_rename_is_reported_and_leaves_the_legacy_file() {
        use std::os::unix::fs::PermissionsExt;

        let directory = temp_config_dir("failed");
        fs::write(directory.join(LEGACY_SETTINGS_FILE_NAME), b"legacy").unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o555)).unwrap();
        // Root ignores directory permissions, so the rename cannot fail there.
        let directory_is_read_only = fs::write(directory.join("probe"), b"").is_err();

        let outcome = migrate_legacy_settings_file_in(&directory);

        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
        if directory_is_read_only {
            match outcome {
                Some(SettingsFileMigration::Failed(report)) => {
                    assert!(report.contains(LEGACY_SETTINGS_FILE_NAME));
                    assert!(report.contains(SETTINGS_FILE_NAME));
                }
                other => panic!("expected a failed migration, got {other:?}"),
            }
            assert_eq!(
                fs::read(directory.join(LEGACY_SETTINGS_FILE_NAME)).unwrap(),
                b"legacy"
            );
            assert!(!directory.join(SETTINGS_FILE_NAME).exists());
        }
        fs::remove_dir_all(&directory).unwrap();
    }
}
