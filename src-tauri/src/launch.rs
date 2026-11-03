//! Command-line options: parsed once in `main`, and again for the arguments a
//! second launch hands to the running instance.

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "somedrive",
    version,
    about = "Linux-first desktop client for Microsoft OneDrive"
)]
struct Cli {
    /// Start hidden in the system tray (used by autostart)
    #[arg(short, long)]
    start_hidden: bool,

    /// Set by the in-app updater when it relaunches this version because the
    /// install failed
    #[arg(long, hide = true)]
    update_failed: bool,
}

/// How this instance was launched.
pub struct LaunchOptions {
    /// Autostart: stay in the tray.
    pub start_hidden: bool,
    /// The in-app updater relaunched this version because the install failed.
    pub update_failed: bool,
}

/// Parses this process's arguments; `--version` and `--help` print and exit.
pub fn parse_launch_options() -> LaunchOptions {
    let cli = Cli::parse();
    LaunchOptions {
        start_hidden: cli.start_hidden,
        update_failed: cli.update_failed,
    }
}

/// Whether a launch should show the main window: every launch does, except
/// autostart's `--start-hidden`. Arguments that fail to parse still show it.
pub fn launch_shows_window(args: &[String]) -> bool {
    Cli::try_parse_from(args)
        .map(|cli| !cli.start_hidden)
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::launch_shows_window;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn plain_launch_shows_window() {
        assert!(launch_shows_window(&args(&["somedrive"])));
    }

    #[test]
    fn start_hidden_launch_keeps_window_hidden() {
        assert!(!launch_shows_window(&args(&[
            "somedrive",
            "--start-hidden"
        ])));
        assert!(!launch_shows_window(&args(&["somedrive", "-s"])));
    }

    #[test]
    fn unparseable_launch_shows_window() {
        assert!(launch_shows_window(&args(&["somedrive", "--unknown-flag"])));
    }
}
