//! Owns the tray icon, tray availability, and every way the main window is
//! shown, parked or quit (Release and Update Standard, section 11). SomeDrive
//! ships for Linux only, so the tray is a StatusNotifierItem
//! (`status_notifier`):
//! - left click opens the main window;
//! - right click shows the menu (Open SomeDrive / Pause all syncing / Resume
//!   all syncing / Quit).

mod status_notifier;

use crate::app::app_settings::load_close_to_tray;
use crate::app::commands::{pause_all_accounts, resume_all_accounts};
use serde::Serialize;
use tauri::{Emitter, Manager};

/// Emitted after the tray changes account sync state, so the UI refreshes.
pub const ACCOUNTS_UPDATED_EVENT: &str = "accounts-updated";

/// Where the main window went when it was taken off screen without quitting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowParking {
    Tray,
    Taskbar,
}

/// The tray menu, in display order. The tray builds its menu from this list
/// and routes selections through `handle_menu_item`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayMenuItem {
    Open,
    PauseAll,
    ResumeAll,
    Quit,
}

impl TrayMenuItem {
    const ALL: [Self; 4] = [Self::Open, Self::PauseAll, Self::ResumeAll, Self::Quit];

    fn label(self) -> &'static str {
        match self {
            Self::Open => "Open SomeDrive",
            Self::PauseAll => "Pause all syncing",
            Self::ResumeAll => "Resume all syncing",
            Self::Quit => "Quit",
        }
    }

    /// Stable id for logs (the label is user-facing text).
    fn id(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::PauseAll => "pause_all",
            Self::ResumeAll => "resume_all",
            Self::Quit => "quit",
        }
    }

    #[cfg(test)]
    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|item| item.id() == id)
    }
}

fn handle_menu_item(app_handle: &tauri::AppHandle, item: TrayMenuItem) {
    log::info!("Tray menu item selected id={}", item.id());
    match item {
        TrayMenuItem::Open => show_main_window(app_handle),
        TrayMenuItem::PauseAll => {
            report_sync_change(app_handle, pause_all_accounts(app_handle.state()))
        }
        TrayMenuItem::ResumeAll => {
            report_sync_change(app_handle, resume_all_accounts(app_handle.state()))
        }
        TrayMenuItem::Quit => quit(app_handle),
    }
}

fn report_sync_change(app_handle: &tauri::AppHandle, result: Result<u32, String>) {
    match result {
        Ok(changed_accounts) => {
            log::info!("Tray sync change applied accounts={changed_accounts}");
            if let Err(error) = app_handle.emit(ACCOUNTS_UPDATED_EVENT, ()) {
                log::warn!("Failed to notify the UI of the tray sync change: {error}");
            }
        }
        Err(error) => log::warn!("Tray sync change failed: {error}"),
    }
}

/// Creates the tray icon.
pub fn build_tray(app: &tauri::App<tauri::Wry>) -> Result<(), Box<dyn std::error::Error>> {
    let icon = app
        .default_window_icon()
        .cloned()
        .map(tauri::image::Image::to_owned)
        .ok_or("SomeDrive has no bundled icon to show in the system tray")?;
    status_notifier::spawn(app.handle(), &icon);
    Ok(())
}

/// Routes every close request (title-bar button, Alt+F4, taskbar "Close
/// window") through `close_main_window` so the `close_to_tray` setting is
/// honored no matter how the window was closed.
pub fn intercept_main_window_close(app: &tauri::App<tauri::Wry>) {
    let Some(window) = app.get_webview_window("main") else {
        log::warn!("Main window not found; close-to-tray handling not installed");
        return;
    };
    let app_handle = app.handle().clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let app_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = close_main_window(&app_handle).await {
                    log::warn!("Failed to close main window: {}", error);
                }
            });
        }
    });
}

/// Whether a system tray host is present to receive the tray icon: a
/// StatusNotifier watcher (absent on stock GNOME without the AppIndicator
/// extension).
pub async fn is_tray_available() -> bool {
    status_notifier::is_watcher_available().await
}

/// Takes the main window off screen without quitting: hidden to the tray
/// when one exists, otherwise minimized to the taskbar so it stays reachable.
pub async fn park_main_window(app_handle: &tauri::AppHandle) -> Result<WindowParking, String> {
    let window = app_handle
        .get_webview_window("main")
        .ok_or_else(|| "Main window not found".to_string())?;

    if is_tray_available().await {
        window.hide().map_err(|error| error.to_string())?;
        return Ok(WindowParking::Tray);
    }

    window.minimize().map_err(|error| error.to_string())?;
    Ok(WindowParking::Taskbar)
}

/// Closes the main window according to the `close_to_tray` setting. Returns
/// where the window was parked, or `None` when the application is quitting.
pub async fn close_main_window(
    app_handle: &tauri::AppHandle,
) -> Result<Option<WindowParking>, String> {
    let close_to_tray = load_close_to_tray().unwrap_or_else(|error| {
        log::warn!("Failed to read close-to-tray setting, using the default: {error}");
        true
    });

    if !close_to_tray {
        quit(app_handle);
        return Ok(None);
    }

    let parking = park_main_window(app_handle).await?;
    if parking == WindowParking::Taskbar {
        log::warn!(
            "Close to tray requested but no system tray is available; minimized to taskbar instead"
        );
    }
    Ok(Some(parking))
}

/// The single path for bringing SomeDrive forward: tray click, tray menu and
/// a second launch all end here.
pub fn show_main_window(app_handle: &tauri::AppHandle) {
    let Some(window) = app_handle.get_webview_window("main") else {
        log::warn!("Cannot show the main window: window not found");
        return;
    };
    if let Err(error) = window.unminimize() {
        log::warn!("Failed to unminimize the main window: {}", error);
    }
    if let Err(error) = window.show() {
        log::warn!("Failed to show the main window: {}", error);
    }
    if let Err(error) = window.set_focus() {
        log::warn!("Failed to focus the main window: {}", error);
    }
}

/// A second launch (app menu, desktop file, autostart) hands its arguments
/// to this instance and exits. Bring the window forward unless that launch
/// asked to start hidden.
pub fn handle_second_launch(app_handle: &tauri::AppHandle, show_window: bool) {
    if show_window {
        log::info!("Second launch detected; showing the main window");
        show_main_window(app_handle);
    } else {
        log::info!("Second launch detected with --start-hidden; leaving the window as is");
    }
}

/// Exits the app; the `RunEvent::Exit` handler drains sync first
/// (`app::shutdown::finish_before_exit`).
pub fn quit(app_handle: &tauri::AppHandle) {
    app_handle.exit(0);
}

#[cfg(test)]
mod tests {
    use super::TrayMenuItem;

    #[test]
    fn menu_lists_open_first_and_quit_last() {
        assert_eq!(TrayMenuItem::ALL.first(), Some(&TrayMenuItem::Open));
        assert_eq!(TrayMenuItem::ALL.last(), Some(&TrayMenuItem::Quit));
    }

    #[test]
    fn menu_item_ids_round_trip() {
        for item in TrayMenuItem::ALL {
            assert_eq!(TrayMenuItem::from_id(item.id()), Some(item));
        }
        assert_eq!(TrayMenuItem::from_id("unknown"), None);
    }
}
