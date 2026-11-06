import { invoke } from "@tauri-apps/api/core";
import { isEnabled as isAutostartEnabled } from "@tauri-apps/plugin-autostart";
import type {
  AppStatusSnapshot,
  CleanupView,
  ConflictPolicy,
  DownloadConcurrencySetting,
  LogLevel,
  SyncProblemsView,
  SyncRuntimeSnapshot,
  SystemInfo,
} from "../types/somedrive";

// Every backend read the window makes (docs/ui-data-flow.md). Only the store
// in `src/state/` calls these; `npm run check:ui-data-reads` fails the build
// if a read command is invoked anywhere else. Screens send actions (delete,
// pause, save a setting) and the store hears about the result through the
// change counters.

/** What Settings shows, saved in `settings.json` (plus auto-start, which
 *  the desktop holds). */
export interface AppSettingsView {
  autostartEnabled: boolean;
  rawLoggerMode: boolean;
  closeToTray: boolean;
  logLevel: LogLevel;
  conflictPolicy: ConflictPolicy;
  excludePatterns: string[];
  syncDownloadConcurrency: DownloadConcurrencySetting;
}

export const readStatusSnapshot = () => invoke<AppStatusSnapshot>("get_status_snapshot");

export const readSyncRuntime = () => invoke<SyncRuntimeSnapshot>("get_sync_runtime_snapshot");

/** Asks the backend to send every account's status as `sync-status` events. */
export const requestSyncStatusSnapshot = () => invoke<void>("request_sync_status_snapshot");

export async function readSettings(): Promise<AppSettingsView> {
  const [
    autostartEnabled,
    rawLoggerMode,
    closeToTray,
    logLevel,
    conflictPolicy,
    excludePatterns,
    syncDownloadConcurrency,
  ] = await Promise.all([
    isAutostartEnabled(),
    invoke<boolean>("get_raw_logger_mode"),
    invoke<boolean>("get_close_to_tray"),
    invoke<LogLevel>("get_log_level"),
    invoke<ConflictPolicy>("get_conflict_policy"),
    invoke<string[]>("get_exclude_patterns"),
    invoke<DownloadConcurrencySetting>("get_sync_download_concurrency"),
  ]);
  return {
    autostartEnabled,
    rawLoggerMode,
    closeToTray,
    logLevel,
    conflictPolicy,
    excludePatterns,
    syncDownloadConcurrency,
  };
}

export const readSystemInfo = () => invoke<SystemInfo>("get_system_info");

export const readSyncProblems = (profileId: string) => invoke<SyncProblemsView>("list_sync_problems", { profileId });

export const readCleanup = (profileId: string) => invoke<CleanupView>("list_cleanup", { profileId });

export const readLargeDeletePreview = (profileId: string) =>
  invoke<string[]>("get_account_large_delete_preview", { profileId });

/** The session log as text, read on demand for the Debug Tools preview. */
export const readSessionLogText = () => invoke<string>("get_session_log_text");
