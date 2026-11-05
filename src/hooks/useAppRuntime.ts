import { useEffect, useMemo, useState } from "preact/hooks";
import { routeStateFromHash, type AppRouteState } from "../routes/appRoutes";
import type { SyncRuntimeAccountStatus, ToastType } from "../types/somedrive";
import { computeEffectiveSyncState } from "../components/accounts/syncStateSelectors";
import { useAppData } from "../state/useAppData";
import { createAccountActions } from "./appRuntime/accountActions";
import { createCleanupActions } from "./appRuntime/cleanupActions";
import { createNavigationActions } from "./appRuntime/navigation";
import { createSystemActions } from "./appRuntime/systemActions";

interface UseAppRuntimeProps {
  showToast: (message: string, type?: ToastType) => void;
}

/**
 * Composes the app: what it shows comes from the store (`src/state/`),
 * which stays current by itself; this adds navigation and the actions
 * screens send. Actions never reload anything: their effect reaches the
 * store through the backend's change counters.
 */
export function useAppRuntime({ showToast }: UseAppRuntimeProps) {
  const [routeState, setRouteState] = useState<AppRouteState>(routeStateFromHash(window.location.hash));
  const data = useAppData({ showToast });

  const [accountToOpen, setAccountToOpen] = useState<string | null>(null);

  const navigation = useMemo(() => createNavigationActions({ setRouteState }), [setRouteState]);
  const accountActions = useMemo(
    () => createAccountActions({ showToast, openAccountWhenListed: setAccountToOpen }),
    [showToast]
  );

  // A new account's page opens once the store lists it, never before.
  useEffect(() => {
    if (accountToOpen && data.status.accounts.some((account) => account.id === accountToOpen)) {
      navigation.openAccount(accountToOpen);
      setAccountToOpen(null);
    }
  }, [accountToOpen, data.status.accounts, navigation]);
  const cleanupActions = useMemo(() => createCleanupActions({ showToast }), [showToast]);
  const systemActions = useMemo(() => createSystemActions({ showToast }), [showToast]);

  useEffect(() => {
    const syncRoute = () => setRouteState(routeStateFromHash(window.location.hash));
    window.addEventListener("hashchange", syncRoute);
    return () => window.removeEventListener("hashchange", syncRoute);
  }, []);

  const syncRuntimeByAccountId = useMemo<Record<string, SyncRuntimeAccountStatus>>(
    () => Object.fromEntries(data.syncRuntime.accounts.map((status) => [status.profileId, status])),
    [data.syncRuntime.accounts]
  );

  const syncCounts = useMemo(() => {
    let syncing = 0;
    let paused = 0;
    for (const account of data.status.accounts) {
      const runtimeStatus = syncRuntimeByAccountId[account.id] ?? null;
      const { syncState } = computeEffectiveSyncState(account, runtimeStatus);
      if (syncState === "syncing") {
        syncing += 1;
      } else if (syncState === "paused") {
        paused += 1;
      }
    }
    return { syncing, paused };
  }, [data.status.accounts, syncRuntimeByAccountId]);

  const settings = data.settings;
  return {
    routeState,
    status: data.status,
    syncRuntime: data.syncRuntime,
    syncingCount: syncCounts.syncing,
    pausedCount: syncCounts.paused,
    navigate: navigation.navigate,
    goHome: navigation.goHome,
    openAccount: navigation.openAccount,
    openAccountSettings: navigation.openAccountSettings,
    goSettings: navigation.goSettings,
    goDebug: navigation.goDebug,
    goUiLab: navigation.goUiLab,
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
    resolveSyncConflicts: accountActions.resolveSyncConflicts,
    confirmAccountLargeDelete: accountActions.confirmAccountLargeDelete,
    keepCloudFilesAfterLargeDelete: accountActions.keepCloudFilesAfterLargeDelete,
    exportAccountLargeDeletePreview: accountActions.exportAccountLargeDeletePreview,
    cleanupActions,
    autostartEnabled: settings?.autostartEnabled ?? false,
    toggleAutostart: data.setAutostartEnabled,
    closeToTray: settings?.closeToTray ?? true,
    toggleCloseToTray: systemActions.setCloseToTray,
    rawLoggerMode: settings?.rawLoggerMode ?? false,
    toggleRawLoggerMode: systemActions.setRawLoggerMode,
    logLevel: settings?.logLevel ?? "info",
    changeLogLevel: systemActions.setLogLevel,
    systemInfo: data.systemInfo,
    openLogFolder: systemActions.openLogFolder,
    copySystemInfo: systemActions.copySystemInfo,
    syncDownloadConcurrency: settings?.syncDownloadConcurrency ?? null,
    updateSyncDownloadConcurrency: systemActions.setSyncDownloadConcurrency,
    conflictPolicy: settings?.conflictPolicy ?? null,
    updateConflictPolicy: systemActions.setConflictPolicy,
    excludePatterns: settings?.excludePatterns ?? null,
    updateExcludePatterns: systemActions.setExcludePatterns,
    fetchSessionLogText: data.fetchSessionLogText,
    copySessionLog: systemActions.copySessionLog,
    openSessionLog: systemActions.openSessionLog,
    openProfileLog: systemActions.openProfileLog,
    openAccountSyncRootFolder: systemActions.openAccountSyncRootFolder,
    openAccountItemFolder: systemActions.openAccountItemFolder,
  };
}
