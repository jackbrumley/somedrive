import { AppPageRenderer } from "./AppPageRenderer";
import { AccountsHomePage } from "../pages/AccountsHomePage";
import { AccountDetailPage } from "../pages/AccountDetailPage";
import { SettingsPage } from "../pages/SettingsPage";
import { DebugPage } from "../pages/DebugPage";
import { UiLabPage } from "../pages/UiLabPage";
import type { useAppRuntime } from "../hooks/useAppRuntime";
import type { useUpdates } from "../hooks/useUpdates";

type AppRuntime = ReturnType<typeof useAppRuntime>;
type AppUpdates = ReturnType<typeof useUpdates>;

interface AppWorkspaceProps {
  runtime: AppRuntime;
  updates: AppUpdates;
}

export function AppWorkspace({ runtime, updates }: AppWorkspaceProps) {
  const selectedAccount = runtime.routeState.accountId
    ? runtime.status.accounts.find((account) => account.id === runtime.routeState.accountId) ?? null
    : null;

  const syncRuntimeByAccountId = Object.fromEntries(
    runtime.syncRuntime.accounts.map((status) => [status.profileId, status])
  );

  return (
    <AppPageRenderer
      page={runtime.routeState.page}
      renderAccountsHome={() => (
        <AccountsHomePage
          accounts={runtime.status.accounts}
          appVersion={runtime.status.appVersion}
          syncRuntimeByAccountId={syncRuntimeByAccountId}
          onCreateAccount={runtime.createAccountProfile}
          onOpenAccount={runtime.openAccount}
          onSetAgentState={runtime.setAccountAgentState}
          onOpenSyncRootFolder={runtime.openAccountSyncRootFolder}
        />
      )}
      renderAccountDetail={() => (
        <AccountDetailPage
          account={selectedAccount}
          runtimeStatus={selectedAccount ? (syncRuntimeByAccountId[selectedAccount.id] ?? null) : null}
          view={runtime.routeState.accountView ?? "sync"}
          onBack={runtime.goHome}
          onOpenSettings={runtime.openAccountSettings}
          onOpenSync={runtime.openAccount}
          onSetAgentState={runtime.setAccountAgentState}
          onStartAuth={runtime.startInteractiveAuth}
          onRename={runtime.renameAccountProfile}
          onSetSyncRoot={runtime.setAccountSyncRoot}
          onClearAuth={runtime.clearAccountAuth}
          onRemoveProfile={runtime.removeAccountProfile}
          onOpenSyncRootFolder={runtime.openAccountSyncRootFolder}
          onOpenItemFolder={runtime.openAccountItemFolder}
          onReauthenticate={runtime.startInteractiveAuth}
          onRetrySync={runtime.retryAccountSync}
          onRetryFailedDownload={runtime.retryFailedDownload}
          onRetryAllFailedDownloads={runtime.retryAllFailedDownloads}
          onConfirmLargeDelete={runtime.confirmAccountLargeDelete}
          onKeepCloudFiles={runtime.keepCloudFilesAfterLargeDelete}
          onFetchLargeDeletePreview={runtime.fetchAccountLargeDeletePreview}
          onExportLargeDeletePreview={runtime.exportAccountLargeDeletePreview}
          onFetchSyncProblems={runtime.fetchSyncProblems}
          onResolveSyncConflicts={runtime.resolveSyncConflicts}
        />
      )}
      renderSettings={() => (
        <SettingsPage
          autostartEnabled={runtime.autostartEnabled}
          onToggleAutostart={runtime.toggleAutostart}
          closeToTray={runtime.closeToTray}
          onToggleCloseToTray={runtime.toggleCloseToTray}
          rawLoggerMode={runtime.rawLoggerMode}
          onToggleRawLoggerMode={runtime.toggleRawLoggerMode}
          syncDownloadConcurrency={runtime.syncDownloadConcurrency}
          onChangeSyncDownloadConcurrency={runtime.updateSyncDownloadConcurrency}
          conflictPolicy={runtime.conflictPolicy}
          onChangeConflictPolicy={runtime.updateConflictPolicy}
          appVersion={runtime.status.appVersion}
          checkingUpdates={updates.checkingUpdates}
          lastCheckedAt={updates.lastCheckedAt}
          updateError={updates.updateError}
          onCheckForUpdates={() => void updates.checkForUpdates(true)}
          systemInfo={runtime.systemInfo}
          logLevel={runtime.logLevel}
          onChangeLogLevel={runtime.changeLogLevel}
          onCopyLog={runtime.copySessionLog}
          onOpenLog={runtime.openSessionLog}
          onOpenLogFolder={runtime.openLogFolder}
          onCopySystemInfo={runtime.copySystemInfo}
          onGoDebug={runtime.goDebug}
          onBack={runtime.goHome}
        />
      )}
      renderDebug={() => (
        <DebugPage
          status={runtime.status}
          onBack={runtime.goSettings}
          onNavigateUiLab={runtime.goUiLab}
          onRefreshStatus={runtime.refreshStatus}
          onFetchSessionLogText={runtime.fetchSessionLogText}
          onOpenProfileLog={runtime.openProfileLog}
        />
      )}
      renderUiLab={() => <UiLabPage onBack={runtime.goDebug} />}
    />
  );
}
