import { invoke } from "@tauri-apps/api/core";
import type { ConflictPolicy, DownloadConcurrencySetting, LogLevel, ToastType } from "../../types/somedrive";

interface SystemActionsFactoryParams {
  showToast: (message: string, type?: ToastType) => void;
}

/** Settings and log actions. A saved setting reaches the window through the
 *  store (`app-data-changed`), so nothing here keeps or returns state. */
export function createSystemActions({ showToast }: SystemActionsFactoryParams) {
  const run = async (action: () => Promise<unknown>, failure: string, success?: string) => {
    try {
      await action();
      if (success) {
        showToast(success, "success");
      }
    } catch (error) {
      showToast(`${failure}: ${error}`, "error");
    }
  };

  return {
    copySessionLog: () =>
      run(() => invoke("copy_session_log"), "Failed to copy session log", "Session log copied to clipboard."),
    openSessionLog: () => run(() => invoke("open_session_log"), "Failed to open session log"),
    openLogFolder: () => run(() => invoke("open_log_folder"), "Failed to open log folder"),
    copySystemInfo: () =>
      run(() => invoke("copy_system_info"), "Failed to copy system info", "System info copied to clipboard."),
    openProfileLog: (profileId: string) =>
      run(() => invoke("open_profile_log", { profileId }), "Failed to open profile log"),
    setLogLevel: (level: LogLevel) =>
      run(() => invoke<LogLevel>("set_log_level", { level }), "Failed to update log level", `Log level set to ${level}.`),
    setCloseToTray: (enabled: boolean) =>
      run(
        () => invoke<boolean>("set_close_to_tray", { enabled }),
        "Failed to update close-to-tray setting",
        `Close to tray ${enabled ? "enabled" : "disabled"}.`
      ),
    setRawLoggerMode: (enabled: boolean) =>
      run(
        () => invoke("set_raw_logger_mode", { enabled }),
        "Failed to update raw logger mode",
        `Raw logger mode ${enabled ? "enabled" : "disabled"}.`
      ),
    setSyncDownloadConcurrency: (value: number) =>
      run(
        () => invoke<DownloadConcurrencySetting>("set_sync_download_concurrency", { value }),
        "Failed to update download concurrency",
        `Download concurrency set to ${value}.`
      ),
    setExcludePatterns: (patterns: string[]) =>
      run(
        () => invoke<string[]>("set_exclude_patterns", { patterns }),
        "Couldn't save the exclude rules",
        "Exclude rules saved. They apply from the next sync."
      ),
    setConflictPolicy: (policy: ConflictPolicy) =>
      run(
        () => invoke<ConflictPolicy>("set_conflict_policy", { policy }),
        "Failed to update the conflict setting",
        policy === "ask"
          ? "Files changed on both sides will wait for your decision."
          : "Files changed on both sides will keep the newest version."
      ),
    openAccountItemFolder: (profileId: string, relativePath: string) =>
      run(
        () => invoke("open_account_item_folder", { input: { profileId, relativePath } }),
        "Failed to open folder"
      ),
    openAccountSyncRootFolder: (profileId: string) =>
      run(() => invoke("open_account_sync_root_folder", { profileId }), "Failed to open folder"),
  };
}
