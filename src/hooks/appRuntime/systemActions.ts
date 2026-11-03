import { invoke } from "@tauri-apps/api/core";
import {
  disable as disableAutostart,
  enable as enableAutostart,
  isEnabled as isAutostartEnabled,
} from "@tauri-apps/plugin-autostart";
import type { ToastType } from "../../types/somedrive";

interface SystemActionsFactoryParams {
  showToast: (message: string, type?: ToastType, durationMs?: number) => void;
}

export function createSystemActions({ showToast }: SystemActionsFactoryParams) {
  const getAutostartEnabled = async () => {
    try {
      return await isAutostartEnabled();
    } catch (error) {
      showToast(`Failed to read auto-start state: ${error}`, "error", 4200);
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
      showToast(`Auto-start ${enabled ? "enabled" : "disabled"}.`, "success", 2600);
      return enabled;
    } catch (error) {
      showToast(`Failed to update auto-start: ${error}`, "error", 4200);
      return null;
    }
  };

  const fetchSessionLogText = async () => {
    try {
      return await invoke<string>("get_session_log_text");
    } catch (error) {
      showToast(`Failed to read session log: ${error}`, "error", 4200);
      return "";
    }
  };

  const copySessionLog = async () => {
    try {
      await invoke("copy_session_log");
      showToast("Session log copied to clipboard.", "success", 2600);
    } catch (error) {
      showToast(`Failed to copy session log: ${error}`, "error", 4200);
    }
  };

  const openSessionLog = async () => {
    try {
      await invoke("open_session_log");
    } catch (error) {
      showToast(`Failed to open session log: ${error}`, "error", 4200);
    }
  };

  const openProfileLog = async (profileId: string) => {
    try {
      await invoke("open_profile_log", { profileId });
    } catch (error) {
      showToast(`Failed to open profile log: ${error}`, "error", 4200);
    }
  };

  const fetchCloseToTray = async () => {
    try {
      return await invoke<boolean>("get_close_to_tray");
    } catch (error) {
      showToast(`Failed to read close-to-tray setting: ${error}`, "error", 4200);
      return true;
    }
  };

  const setCloseToTray = async (enabled: boolean) => {
    try {
      const updated = await invoke<boolean>("set_close_to_tray", { enabled });
      showToast(`Close to tray ${updated ? "enabled" : "disabled"}.`, "success", 2600);
      return updated;
    } catch (error) {
      showToast(`Failed to update close-to-tray setting: ${error}`, "error", 4200);
      return null;
    }
  };

  const fetchRawLoggerMode = async () => {
    try {
      return await invoke<boolean>("get_raw_logger_mode");
    } catch (error) {
      showToast(`Failed to read raw logger mode: ${error}`, "error", 4200);
      return false;
    }
  };

  const setRawLoggerMode = async (enabled: boolean) => {
    try {
      await invoke("set_raw_logger_mode", { enabled });
      showToast(`Raw logger mode ${enabled ? "enabled" : "disabled"}.`, "success", 2600);
      return enabled;
    } catch (error) {
      showToast(`Failed to update raw logger mode: ${error}`, "error", 4200);
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
      showToast(`Failed to open folder: ${error}`, "error", 3200);
    }
  };

  const openAccountSyncRootFolder = async (profileId: string) => {
    try {
      await invoke("open_account_sync_root_folder", {
        profileId,
      });
    } catch (error) {
      showToast(`Failed to open folder: ${error}`, "error", 3200);
    }
  };

  const fetchSyncDownloadConcurrency = async () => {
    try {
      return await invoke<number>("get_sync_download_concurrency");
    } catch (error) {
      showToast(`Failed to read download concurrency setting: ${error}`, "error", 4200);
      return 12;
    }
  };

  const setSyncDownloadConcurrency = async (value: number) => {
    try {
      const updated = await invoke<number>("set_sync_download_concurrency", {
        value,
      });
      showToast(`Download concurrency set to ${updated}.`, "success", 2200);
      return updated;
    } catch (error) {
      showToast(`Failed to update download concurrency: ${error}`, "error", 4200);
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
    fetchCloseToTray,
    setCloseToTray,
    fetchRawLoggerMode,
    setRawLoggerMode,
    fetchSyncDownloadConcurrency,
    setSyncDownloadConcurrency,
    openAccountItemFolder,
    openAccountSyncRootFolder,
  };
}
