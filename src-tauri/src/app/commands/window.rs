use crate::app::app_settings::{load_close_to_tray, save_close_to_tray};
use crate::app::tray::{self, WindowParking};

/// Title-bar minimise button: parks the window (tray, else taskbar).
#[tauri::command]
pub async fn minimize_to_tray_or_taskbar(
    app_handle: tauri::AppHandle,
) -> Result<WindowParking, String> {
    tray::park_main_window(&app_handle).await
}

/// Title-bar close button. Returns where the window was parked, or `None`
/// when the `close_to_tray` setting is off and the application is quitting.
#[tauri::command]
pub async fn close_main_window(
    app_handle: tauri::AppHandle,
) -> Result<Option<WindowParking>, String> {
    tray::close_main_window(&app_handle).await
}

#[tauri::command]
pub fn get_close_to_tray() -> Result<bool, String> {
    load_close_to_tray()
}

#[tauri::command]
pub fn set_close_to_tray(enabled: bool) -> Result<bool, String> {
    save_close_to_tray(enabled)?;
    log::info!("WINDOW_SETTING_UPDATED key=close_to_tray value={enabled}");
    Ok(enabled)
}
