import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useRef, useState } from "preact/hooks";
import type { ToastType, UpdateCheckResult, UpdateInstallStage } from "../types/somedrive";

interface UseUpdatesProps {
  showToast: (message: string, type?: ToastType) => void;
}

const logUpdateEvent = (message: string) => {
  void invoke("log_ui_event", { message }).catch(() => {
    // no-op
  });
};

// In-app updates per the Release and Update Standard: a silent check on
// startup, a manual check from Settings, and an update modal whose Update Now
// re-runs the one-line installer (the backend first finishes in-flight sync).
export function useUpdates({ showToast }: UseUpdatesProps) {
  const [checkingUpdates, setCheckingUpdates] = useState(false);
  const [updateResult, setUpdateResult] = useState<UpdateCheckResult | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [lastCheckedAt, setLastCheckedAt] = useState<number | null>(null);
  const [showUpdateModal, setShowUpdateModal] = useState(false);
  const [installStage, setInstallStage] = useState<UpdateInstallStage>("idle");
  const [installError, setInstallError] = useState<string | null>(null);
  const checkingRef = useRef(false);

  const checkForUpdates = async (manual: boolean) => {
    if (checkingRef.current) {
      return;
    }
    checkingRef.current = true;
    setCheckingUpdates(true);
    setUpdateError(null);
    try {
      const result = await invoke<UpdateCheckResult>("check_for_updates");
      setUpdateResult(result);
      if (result.updateAvailable) {
        setInstallError(null);
        setShowUpdateModal(true);
      } else if (manual) {
        showToast("You are already on the latest version.", "success");
      }
    } catch (error) {
      const message = String(error);
      setUpdateError(message);
      if (manual) {
        showToast(`Update check failed: ${message}`, "error");
      } else {
        logUpdateEvent(`Startup update check failed: ${message}`);
      }
    } finally {
      setLastCheckedAt(Date.now());
      checkingRef.current = false;
      setCheckingUpdates(false);
    }
  };

  const installUpdate = async () => {
    if (installStage !== "idle") {
      return;
    }
    setInstallError(null);
    setInstallStage("finishing_sync");
    logUpdateEvent(`Installing update to v${updateResult?.latestVersion ?? "latest"}`);
    try {
      // Resolves once sync has stopped cleanly and the installer is running;
      // the backend then exits and the installer relaunches SomeDrive.
      await invoke("install_update");
      setInstallStage("starting");
    } catch (error) {
      const message = String(error);
      setInstallStage("idle");
      setInstallError(message);
      logUpdateEvent(`Failed to start update: ${message}`);
    }
  };

  const openReleaseNotes = () => {
    if (!updateResult) {
      return;
    }
    void openUrl(updateResult.releaseUrl).catch((error) => {
      showToast(`Failed to open release notes: ${error}`, "error");
    });
  };

  const closeUpdateModal = () => {
    if (installStage === "idle") {
      setShowUpdateModal(false);
    }
  };

  useEffect(() => {
    void checkForUpdates(false);
  }, []);

  return {
    checkingUpdates,
    updateResult,
    updateError,
    lastCheckedAt,
    showUpdateModal,
    installStage,
    installError,
    checkForUpdates,
    installUpdate,
    openReleaseNotes,
    closeUpdateModal,
  };
}
