import { invoke } from "@tauri-apps/api/core";
import {
  disable as disableAutostart,
  enable as enableAutostart,
  isEnabled as isAutostartEnabled,
} from "@tauri-apps/plugin-autostart";
import type {
  ConflictPolicy,
  DownloadConcurrencySetting,
  LogLevel,
  SystemInfo,
  ToastType,
} from "../../types/somedrive";

interface SystemActionsFactoryParams {
  showToast: (message: string, type?: ToastType) => void;
}

export function createSystemActions({ showToast }: SystemActionsFactoryParams) {
  const getAutostartEnabled = async () => {
    try {
      return await isAutostartEnabled();
    } catch (error) {
      showToast(`Failed to read auto-start state: ${error}`, "error");
      return false;
    }
  };

  const setAutostartEnabled = async (enabled: boolean) => {
    try {
      if (enabled) {
        await enableAutostart();
      } else {
        await disableAutostart();
      }
      showToast(`Auto-start ${enabled ? "enabled" : "disabled"}.`, "success");
      return enabled;
    } catch (error) {
      showToast(`Failed to update auto-start: ${error}`, "error");
      return null;
    }
  };

  const fetchSessionLogText = async () => {
    try {
      return await invoke<string>("get_session_log_text");
    } catch (error) {
      showToast(`Failed to read session log: ${error}`, "error");
      return "";
    }
  };

  const copySessionLog = async () => {
    try {
      await invoke("copy_session_log");
      showToast("Session log copied to clipboard.", "success");
    } catch (error) {
      showToast(`Failed to copy session log: ${error}`, "error");
    }
  };

  const openSessionLog = async () => {
    try {
      await invoke("open_session_log");
    } catch (error) {
      showToast(`Failed to open session log: ${error}`, "error");
    }
  };

  const openLogFolder = async () => {
    try {
      await invoke("open_log_folder");
    } catch (error) {
      showToast(`Failed to open log folder: ${error}`, "error");
    }
  };

  const fetchSystemInfo = async () => {
    try {
      return await invoke<SystemInfo>("get_system_info");
    } catch (error) {
      showToast(`Failed to read system info: ${error}`, "error");
      return null;
    }
  };

  const copySystemInfo = async () => {
    try {
      await invoke("copy_system_info");
      showToast("System info copied to clipboard.", "success");
    } catch (error) {
      showToast(`Failed to copy system info: ${error}`, "error");
    }
  };

  const fetchLogLevel = async () => {
    try {
      return await invoke<LogLevel>("get_log_level");
    } catch (error) {
      showToast(`Failed to read log level: ${error}`, "error");
      return "info" as LogLevel;
    }
  };

  const setLogLevel = async (level: LogLevel) => {
    try {
      const updated = await invoke<LogLevel>("set_log_level", { level });
      showToast(`Log level set to ${updated}.`, "success");
      return updated;
    } catch (error) {
      showToast(`Failed to update log level: ${error}`, "error");
      return null;
    }
  };

  const openProfileLog = async (profileId: string) => {
    try {
      await invoke("open_profile_log", { profileId });
    } catch (error) {
      showToast(`Failed to open profile log: ${error}`, "error");
    }
  };

  const fetchCloseToTray = async () => {
    try {
      return await invoke<boolean>("get_close_to_tray");
    } catch (error) {
      showToast(`Failed to read close-to-tray setting: ${error}`, "error");
      return true;
    }
  };

  const setCloseToTray = async (enabled: boolean) => {
    try {
      const updated = await invoke<boolean>("set_close_to_tray", { enabled });
      showToast(`Close to tray ${updated ? "enabled" : "disabled"}.`, "success");
      return updated;
    } catch (error) {
      showToast(`Failed to update close-to-tray setting: ${error}`, "error");
      return null;
    }
  };

  const fetchRawLoggerMode = async () => {
    try {
      return await invoke<boolean>("get_raw_logger_mode");
    } catch (error) {
      showToast(`Failed to read raw logger mode: ${error}`, "error");
      return false;
    }
  };

  const setRawLoggerMode = async (enabled: boolean) => {
    try {
      await invoke("set_raw_logger_mode", { enabled });
      showToast(`Raw logger mode ${enabled ? "enabled" : "disabled"}.`, "success");
      return enabled;
    } catch (error) {
      showToast(`Failed to update raw logger mode: ${error}`, "error");
      return null;
    }
  };

  const openAccountItemFolder = async (profileId: string, relativePath: string) => {
    try {
      await invoke("open_account_item_folder", {
        input: {
          profileId,
          relativePath,
        },
      });
    } catch (error) {
      showToast(`Failed to open folder: ${error}`, "error");
    }
  };

  const openAccountSyncRootFolder = async (profileId: string) => {
    try {
      await invoke("open_account_sync_root_folder", {
        profileId,
      });
    } catch (error) {
      showToast(`Failed to open folder: ${error}`, "error");
    }
  };

  const fetchSyncDownloadConcurrency = async () => {
    try {
      return await invoke<DownloadConcurrencySetting>("get_sync_download_concurrency");
    } catch (error) {
      showToast(`Failed to read download concurrency setting: ${error}`, "error");
      return null;
    }
  };

  const fetchExcludePatterns = async (): Promise<string[] | null> => {
    try {
      return await invoke<string[]>("get_exclude_patterns");
    } catch (error) {
      showToast(`Failed to read exclude rules: ${error}`, "error");
      return null;
    }
  };

  const setExcludePatterns = async (patterns: string[]): Promise<string[] | null> => {
    try {
      const saved = await invoke<string[]>("set_exclude_patterns", { patterns });
      showToast("Exclude rules saved. They apply from the next sync.", "success");
      return saved;
    } catch (error) {
      showToast(`${error}`, "error");
      return null;
    }
  };

  const fetchConflictPolicy = async (): Promise<ConflictPolicy | null> => {
    try {
      return await invoke<ConflictPolicy>("get_conflict_policy");
    } catch (error) {
      showToast(`Failed to read the conflict setting: ${error}`, "error");
      return null;
    }
  };

  const setConflictPolicy = async (policy: ConflictPolicy): Promise<ConflictPolicy | null> => {
    try {
      const updated = await invoke<ConflictPolicy>("set_conflict_policy", { policy });
      showToast(
        updated === "ask"
          ? "Files changed on both sides will wait for your decision."
          : "Files changed on both sides will keep the newest version.",
        "success"
      );
      return updated;
    } catch (error) {
      showToast(`Failed to update the conflict setting: ${error}`, "error");
      return null;
    }
  };

  const setSyncDownloadConcurrency = async (value: number) => {
    try {
      const updated = await invoke<DownloadConcurrencySetting>("set_sync_download_concurrency", {
        value,
      });
      showToast(`Download concurrency set to ${updated.value}.`, "success");
      return updated;
    } catch (error) {
      showToast(`Failed to update download concurrency: ${error}`, "error");
      return null;
    }
  };

  return {
    getAutostartEnabled,
    setAutostartEnabled,
    fetchSessionLogText,
    copySessionLog,
    openSessionLog,
    openProfileLog,
    openLogFolder,
    fetchSystemInfo,
    copySystemInfo,
    fetchLogLevel,
    setLogLevel,
    fetchCloseToTray,
    setCloseToTray,
    fetchRawLoggerMode,
    setRawLoggerMode,
    fetchSyncDownloadConcurrency,
    setSyncDownloadConcurrency,
    fetchConflictPolicy,
    setConflictPolicy,
    fetchExcludePatterns,
    setExcludePatterns,
    openAccountItemFolder,
    openAccountSyncRootFolder,
  };
}
