#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Before anything touches GTK, so the Wayland app_id is the app ID.
    #[cfg(target_os = "linux")]
    somedrive_lib::configure_linux_app_identity();

    somedrive_lib::run(somedrive_lib::parse_launch_options())
}
