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
    let cli = Cli::parse();
    somedrive_lib::run(somedrive_lib::LaunchOptions {
        update_failed: cli.update_failed,
    })
}
