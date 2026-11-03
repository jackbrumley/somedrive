//! The single shutdown owner. Every real exit (tray Quit, closing with
//! close-to-tray off, SIGTERM/SIGINT/SIGHUP, an in-app update) ends in
//! `RunEvent::Exit`, which calls `finish_before_exit` so sync is drained
//! before the process ends.

use crate::app::state::AppState;
use crate::app::sync_engine::{self, SyncShutdownReason};
use tauri::Manager;

/// Drains sync before the process exits. If another path (an update) already
/// started the shutdown, waits for it instead of draining twice.
pub fn finish_before_exit(app_handle: &tauri::AppHandle) {
    // Absent only when setup failed before managing the state.
    let Some(state) = app_handle.try_state::<AppState>() else {
        return;
    };
    tauri::async_runtime::block_on(async {
        if sync_engine::begin_sync_shutdown(SyncShutdownReason::Exit).is_ok() {
            if let Err(error) = sync_engine::drain_sync(&state, SyncShutdownReason::Exit).await {
                // Exiting regardless: startup requeues any job left claimed.
                log::warn!("Sync did not stop cleanly before exit: {error}");
            }
        } else if !sync_engine::wait_for_sync_stopped(SyncShutdownReason::Exit.total_timeout())
            .await
        {
            log::warn!("Sync shutdown already under way did not finish before exit");
        }
    });
    log::info!("Application exit");
}

/// SIGTERM (`install.sh`, `kill`, session logout), SIGINT (Ctrl+C) and SIGHUP
/// (terminal closed) quit through the normal exit path, so sync is drained
/// instead of the process dying mid-transfer.
#[cfg(unix)]
pub fn exit_on_termination_signals(app_handle: &tauri::AppHandle) {
    use tokio::signal::unix::{signal, SignalKind};

    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let signals = (
            signal(SignalKind::terminate()),
            signal(SignalKind::interrupt()),
            signal(SignalKind::hangup()),
        );
        let (Ok(mut terminate), Ok(mut interrupt), Ok(mut hangup)) = signals else {
            log::warn!("Failed to install termination signal handlers");
            return;
        };
        let received = tokio::select! {
            _ = terminate.recv() => "SIGTERM",
            _ = interrupt.recv() => "SIGINT",
            _ = hangup.recv() => "SIGHUP",
        };
        log::info!("Received {received}; finishing sync before exiting");
        app_handle.exit(0);
    });
}
