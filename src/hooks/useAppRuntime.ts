import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import { routeStateFromHash, type AppRouteState } from "../routes/appRoutes";
import type {
  DownloadConcurrencySetting,
  ActivityEvent,
  AppStatusSnapshot,
  SyncStatusEvent,
  SyncRuntimeAccountStatus,
  SyncRuntimeSnapshot,
  LogLevel,
  SystemInfo,
  ToastType,
} from "../types/somedrive";
import { computeEffectiveSyncState } from "../components/accounts/syncStateSelectors";
import { createAccountActions } from "./appRuntime/accountActions";
import { createNavigationActions } from "./appRuntime/navigation";
import { createRefreshActions } from "./appRuntime/refresh";
import { createSystemActions } from "./appRuntime/systemActions";

interface UseAppRuntimeProps {
  showToast: (message: string, type?: ToastType) => void;
}

const initialStatus: AppStatusSnapshot = {
  appVersion: "0.0.0",
  platform: "unknown",
  syncEngineReady: false,
  authConfigured: false,
  activeAccount: null,
  lastSyncAt: null,
  health: "degraded",
  accounts: [],
};

const initialSyncRuntime: SyncRuntimeSnapshot = {
  generatedAt: new Date(0).toISOString(),
  revision: 0,
  accounts: [],
};

export function useAppRuntime({ showToast }: UseAppRuntimeProps) {
  const [routeState, setRouteState] = useState<AppRouteState>(routeStateFromHash(window.location.hash));
  const [status, setStatus] = useState<AppStatusSnapshot>(initialStatus);
  const [activityEvents, setActivityEvents] = useState<ActivityEvent[]>([]);
  const [syncRuntime, setSyncRuntime] = useState<SyncRuntimeSnapshot>(initialSyncRuntime);
  const syncStatusSeqByAccountRef = useRef<Record<string, number>>({});
  const hasReceivedSyncStatusRef = useRef(false);
  const [autostartEnabled, setAutostartEnabled] = useState(false);
  const [rawLoggerMode, setRawLoggerMode] = useState(false);
  const [closeToTray, setCloseToTray] = useState(true);
  const [logLevel, setLogLevel] = useState<LogLevel>("info");
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null);
  const [syncDownloadConcurrency, setSyncDownloadConcurrency] =
    useState<DownloadConcurrencySetting | null>(null);

  const navigation = useMemo(() => createNavigationActions({ setRouteState }), [setRouteState]);
  const refreshActions = useMemo(
    () =>
      createRefreshActions({
        showToast,
        setStatus,
        setActivityEvents,
        setSyncRuntime,
      }),
    [showToast]
  );
  const accountActions = useMemo(
    () =>
      createAccountActions({
        showToast,
        refreshStatus: refreshActions.refreshStatus,
        refreshActivity: refreshActions.refreshActivity,
        openAccount: navigation.openAccount,
      }),
    [navigation.openAccount, refreshActions.refreshActivity, refreshActions.refreshStatus, showToast]
  );
  const systemActions = useMemo(() => createSystemActions({ showToast }), [showToast]);

  useEffect(() => {
    const syncRoute = () => setRouteState(routeStateFromHash(window.location.hash));
    let isDisposed = false;
    let bootstrapTimerId: number | null = null;
    const unlistenAuthUpdatePromise = listen("account-auth-updated", () => {
      if (!isDisposed) {
        void Promise.all([refreshActions.refreshStatus(), refreshActions.refreshActivity()]);
      }
    });

    // The tray's Pause/Resume all syncing changes account state outside the UI.
    const unlistenAccountsUpdatedPromise = listen("accounts-updated", () => {
      if (!isDisposed) {
        void Promise.all([refreshActions.refreshStatus(), refreshActions.refreshActivity()]);
      }
    });

    const unlistenSyncStatusPromise = listen<SyncStatusEvent>("sync-status", ({ payload }) => {
      if (!payload?.profileId) {
        return;
      }
      hasReceivedSyncStatusRef.current = true;
      const profileId = payload.profileId;
      const incomingSeq = payload.statusSeq ?? 0;
      if (incomingSeq <= 0) {
        return;
      }

      const currentSeq = syncStatusSeqByAccountRef.current[profileId] ?? 0;
      if (incomingSeq <= currentSeq) {
        return;
      }
      if (currentSeq > 0 && incomingSeq > currentSeq + 1) {
        void refreshActions.refreshSyncRuntime().catch(() => {
          // refresh action already surfaces hard failure
        });
      }
      syncStatusSeqByAccountRef.current = {
        ...syncStatusSeqByAccountRef.current,
        [profileId]: incomingSeq,
      };

      setSyncRuntime((current) => {
        const nextAccounts = current.accounts.filter((account) => account.profileId !== profileId);
        if (payload.kind !== "removed" && payload.status) {
          nextAccounts.push(payload.status);
          nextAccounts.sort((left, right) => left.profileId.localeCompare(right.profileId));
        }
        return {
          generatedAt: payload.generatedAt || new Date().toISOString(),
          revision: Math.max(current.revision + 1, incomingSeq),
          accounts: nextAccounts,
        };
      });
    });

    window.addEventListener("hashchange", syncRoute);
    void refreshActions.refreshStatus();
    void refreshActions.refreshActivity();
    void unlistenSyncStatusPromise
      .then(() => {
        if (isDisposed) {
          return;
        }
        return invoke("request_sync_status_snapshot");
      })
      .then(() => {
        if (isDisposed) {
          return;
        }
        bootstrapTimerId = window.setTimeout(() => {
          if (isDisposed || hasReceivedSyncStatusRef.current) {
            return;
          }
          void refreshActions.refreshSyncRuntime().catch(() => {
            // refresh action already surfaces hard failure
          });
        }, 1200);
      })
      .catch(() => {
        if (!isDisposed) {
          showToast("Failed to bootstrap authoritative sync status snapshot.", "error");
        }
      });
    void systemActions.getAutostartEnabled().then(setAutostartEnabled);
    void systemActions.fetchRawLoggerMode().then(setRawLoggerMode);
    void systemActions.fetchCloseToTray().then(setCloseToTray);
    void systemActions.fetchLogLevel().then(setLogLevel);
    void systemActions.fetchSystemInfo().then(setSystemInfo);
    void systemActions.fetchSyncDownloadConcurrency().then(setSyncDownloadConcurrency);
    return () => {
      isDisposed = true;
      window.removeEventListener("hashchange", syncRoute);
      unlistenAuthUpdatePromise
        .then((unlisten) => unlisten())
        .catch(() => {
          // no-op
        });
      unlistenAccountsUpdatedPromise
        .then((unlisten) => unlisten())
        .catch(() => {
          // no-op
        });
      unlistenSyncStatusPromise
        .then((unlisten) => unlisten())
        .catch(() => {
          // no-op
        });
      if (bootstrapTimerId !== null) {
        window.clearTimeout(bootstrapTimerId);
      }
    };
  }, []);

  const syncRuntimeByAccountId = useMemo<Record<string, SyncRuntimeAccountStatus>>(
    () => Object.fromEntries(syncRuntime.accounts.map((status) => [status.profileId, status])),
    [syncRuntime.accounts]
  );

  const syncCounts = useMemo(() => {
    let syncing = 0;
    let paused = 0;
    for (const account of status.accounts) {
      const runtimeStatus = syncRuntimeByAccountId[account.id] ?? null;
      const { syncState } = computeEffectiveSyncState(account, runtimeStatus);
      if (syncState === "syncing") {
        syncing += 1;
      } else if (syncState === "paused") {
        paused += 1;
      }
    }
    return { syncing, paused };
  }, [status.accounts, syncRuntimeByAccountId]);

  const toggleAutostart = async (enabled: boolean) => {
    const updated = await systemActions.setAutostartEnabled(enabled);
    if (typeof updated === "boolean") {
      setAutostartEnabled(updated);
    }
  };

  const toggleCloseToTray = async (enabled: boolean) => {
    const updated = await systemActions.setCloseToTray(enabled);
    if (typeof updated === "boolean") {
      setCloseToTray(updated);
    }
  };

  const changeLogLevel = async (level: LogLevel) => {
    const updated = await systemActions.setLogLevel(level);
    if (updated) {
      setLogLevel(updated);
    }
  };

  const toggleRawLoggerMode = async (enabled: boolean) => {
    const updated = await systemActions.setRawLoggerMode(enabled);
    if (typeof updated === "boolean") {
      setRawLoggerMode(updated);
    }
  };

  const updateSyncDownloadConcurrency = async (value: number) => {
    const updated = await systemActions.setSyncDownloadConcurrency(value);
    if (updated !== null) {
      setSyncDownloadConcurrency(updated);
    }
  };

  return {
    routeState,
    status,
    activityEvents,
    syncRuntime,
    syncingCount: syncCounts.syncing,
    pausedCount: syncCounts.paused,
    navigate: navigation.navigate,
    goHome: navigation.goHome,
    openAccount: navigation.openAccount,
    openAccountSettings: navigation.openAccountSettings,
    goSettings: navigation.goSettings,
    goDebug: navigation.goDebug,
    goUiLab: navigation.goUiLab,
    refreshStatus: refreshActions.refreshStatus,
    refreshActivity: refreshActions.refreshActivity,
    refreshSyncRuntime: refreshActions.refreshSyncRuntime,
    createAccountProfile: accountActions.createAccountProfile,
    renameAccountProfile: accountActions.renameAccountProfile,
    removeAccountProfile: accountActions.removeAccountProfile,
    setAccountAgentState: accountActions.setAccountAgentState,
    setAccountSyncRoot: accountActions.setAccountSyncRoot,
    startInteractiveAuth: accountActions.startInteractiveAuth,
    clearAccountAuth: accountActions.clearAccountAuth,
    pauseAllAccounts: accountActions.pauseAllAccounts,
    resumeAllAccounts: accountActions.resumeAllAccounts,
    retryAccountSync: accountActions.retryAccountSync,
    retryFailedDownload: accountActions.retryFailedDownload,
    retryAllFailedDownloads: accountActions.retryAllFailedDownloads,
    confirmAccountLargeDelete: accountActions.confirmAccountLargeDelete,
    keepCloudFilesAfterLargeDelete: accountActions.keepCloudFilesAfterLargeDelete,
    fetchAccountLargeDeletePreview: accountActions.fetchAccountLargeDeletePreview,
    exportAccountLargeDeletePreview: accountActions.exportAccountLargeDeletePreview,
    autostartEnabled,
    toggleAutostart,
    closeToTray,
    toggleCloseToTray,
    rawLoggerMode,
    toggleRawLoggerMode,
    logLevel,
    changeLogLevel,
    systemInfo,
    openLogFolder: systemActions.openLogFolder,
    copySystemInfo: systemActions.copySystemInfo,
    syncDownloadConcurrency,
    updateSyncDownloadConcurrency,
    fetchSessionLogText: systemActions.fetchSessionLogText,
    copySessionLog: systemActions.copySessionLog,
    openSessionLog: systemActions.openSessionLog,
    openProfileLog: systemActions.openProfileLog,
    openAccountSyncRootFolder: systemActions.openAccountSyncRootFolder,
    openAccountItemFolder: systemActions.openAccountItemFolder,
  };
}
