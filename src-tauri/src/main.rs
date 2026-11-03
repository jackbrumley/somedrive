#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // First, so even a start-up panic is written to the session log.
    somedrive_lib::install_panic_hook();

    // Before Tauri, GTK or any portal call: app identity and WebKitGTK/GDK
    // environment (App Standard section 2).
    #[cfg(target_os = "linux")]
    somedrive_lib::configure_linux_session_environment();

    somedrive_lib::run(somedrive_lib::parse_launch_options())
}
