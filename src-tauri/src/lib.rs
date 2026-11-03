mod app;
mod launch;
mod paths;
mod platform;

#[cfg(target_os = "linux")]
pub use platform::linux::wayland::env::configure_linux_session_environment;

use app::commands::*;
pub use app::session_log::install_panic_hook;
pub use launch::{parse_launch_options, LaunchOptions};

pub fn run(launch: LaunchOptions) {
    tauri::Builder::default()
        // Must be the first plugin: a second launch hands its arguments to the
        // running instance and exits here, before it writes to that
        // instance's logs or starts a second sync engine.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            app::tray::handle_second_launch(app, launch::launch_shows_window(&args));
        }))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("somedrive")
                .arg("--start-hidden")
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(app::state::create_app_state())
        .setup(move |app| app::bootstrap::run_setup(app, launch))
        .invoke_handler(tauri::generate_handler![
            get_status_snapshot,
            get_sync_runtime_snapshot,
            request_sync_status_snapshot,
            get_sync_download_concurrency,
            set_sync_download_concurrency,
            check_for_updates,
            install_update,
            minimize_to_tray_or_taskbar,
            close_main_window,
            get_close_to_tray,
            set_close_to_tray,
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
            app::session_log::copy_session_log,
            app::session_log::open_log_folder,
            app::session_log::get_log_level,
            app::session_log::set_log_level,
            app::system_info::get_system_info,
            app::system_info::copy_system_info,
            app::session_log::open_session_log,
            app::session_log::open_profile_log,
            app::session_log::get_raw_logger_mode,
            app::session_log::set_raw_logger_mode
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // The single shutdown owner: every exit (tray Quit, close with
            // close-to-tray off, a termination signal, an update) passes
            // through here, so sync is drained before the process ends.
            if let tauri::RunEvent::Exit = event {
                app::shutdown::finish_before_exit(app_handle);
            }
        });
}
