#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "somedrive",
    version,
    about = "Linux-first desktop client for Microsoft OneDrive"
)]
struct Cli {
    /// Set by the in-app updater when it relaunches this version because the
    /// install failed
    #[arg(long, hide = true)]
    update_failed: bool,
}

fn main() {
    // Before anything touches GTK, so the Wayland app_id is the app ID.
    #[cfg(target_os = "linux")]
    somedrive_lib::configure_linux_app_identity();

    let cli = Cli::parse();
    somedrive_lib::run(somedrive_lib::LaunchOptions {
        update_failed: cli.update_failed,
    })
}
