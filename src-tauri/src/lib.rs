mod app;

use app::commands::*;
use tauri::Manager;

/// Options parsed from the command line in `main.rs`.
pub struct LaunchOptions {
    /// The in-app updater relaunched this version because the install failed.
    pub update_failed: bool,
}

pub fn run(launch: LaunchOptions) {
    if let Err(error) = app::session_log::initialize_session_logging() {
        eprintln!("Failed to initialize session logging: {}", error);
    }
    if let Err(error) = app::session_log::initialize_app_logger() {
        eprintln!("Failed to initialize app logger: {}", error);
    }

    log::info!(
        "Application launch | version={} platform={}/{}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            log::warn!("Second launch attempt detected; focusing existing window");
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(app::state::create_app_state())
        .setup(move |app| {
            app::bootstrap::configure_shell(app)?;
            if launch.update_failed {
                app::commands::update::show_update_failed_dialog(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_status_snapshot,
            get_sync_runtime_snapshot,
            request_sync_status_snapshot,
            get_sync_download_concurrency,
            set_sync_download_concurrency,
            check_for_updates,
            install_update,
            list_account_profiles,
            create_account_profile,
            rename_account_profile,
            remove_account_profile,
            set_account_agent_state,
            confirm_account_large_delete,
            keep_cloud_files_after_large_delete,
            retry_failed_download,
            retry_all_failed_downloads,
            get_account_large_delete_preview,
            export_account_large_delete_preview,
            pause_all_accounts,
            resume_all_accounts,
            set_account_sync_root,
            open_account_item_folder,
            open_account_sync_root_folder,
            list_activity_events,
            start_device_auth,
            start_interactive_auth,
            poll_device_auth,
            clear_account_auth,
            app::session_log::log_ui_event,
            app::session_log::get_session_log_text,
            app::session_log::copy_session_log_to_clipboard,
            app::session_log::open_session_log,
            app::session_log::open_profile_log,
            app::session_log::get_raw_logger_mode,
            app::session_log::set_raw_logger_mode
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
