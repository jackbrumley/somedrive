import type { CleanupActions, ConflictResolution, SyncRuntimeAccountStatus } from "../../types/somedrive";
import { useAccountAreas } from "../../state/accountAreas";
import { AccountCleanup } from "./AccountCleanup";
import { AccountNotSynced } from "./AccountNotSynced";
import { AccountSyncProblems } from "./AccountSyncProblems";
import { useTransferRates } from "../../hooks/useTransferRates";
import { AccountSyncActivityList } from "./AccountSyncActivityList";
import { AccountSyncIssues } from "./AccountSyncIssues";
import { AccountSyncOverview } from "./AccountSyncOverview";
import { accountTransfers } from "./transferProgress";

interface AccountSyncActivityPanelProps {
  accountId: string;
  runtimeStatus: SyncRuntimeAccountStatus | null;
  hasCompletedInitialSync: boolean;
  issueMessage: string | null;
  issueKind: "auth_required" | "sync_error" | null;
  issueActions: string[];
  issuePath: string | null;
  issueSecondaryPath: string | null;
  onOpenItemFolder: (relativePath: string) => Promise<void>;
  onOpenSyncRootFolder: () => Promise<void>;
  onReauthenticate: () => Promise<unknown>;
  onRetrySync: () => Promise<void>;
  onRetryFailedDownload: (recentItemId: string, path: string) => Promise<void>;
  onRetryAllFailedDownloads: () => Promise<void>;
  onConfirmLargeDelete: () => Promise<void>;
  onKeepCloudFiles: () => Promise<void>;
  largeDeletePreviewPaths: string[];
  onExportLargeDeletePreview: () => Promise<void>;
  onResolveSyncConflicts: (accountId: string, paths: string[], resolution: ConflictResolution) => Promise<void>;
  cleanupActions: CleanupActions;
}

/**
 * The account's sync page: anything that needs the user (above the tabs, so
 * it is never hidden), then the tabs: a plain-language Summary, the files
 * moving now, problems, skipped items, clean-up and details. The header
 * keeps a one-line progress in view on every tab.
 */
export function AccountSyncActivityPanel({
  accountId,
  runtimeStatus,
  hasCompletedInitialSync,
  issueMessage,
  issueKind,
  issueActions,
  issuePath,
  issueSecondaryPath,
  onOpenItemFolder,
  onOpenSyncRootFolder,
  onReauthenticate,
  onRetrySync,
  onRetryFailedDownload,
  onRetryAllFailedDownloads,
  onConfirmLargeDelete,
  onKeepCloudFiles,
  largeDeletePreviewPaths,
  onExportLargeDeletePreview,
  onResolveSyncConflicts,
  cleanupActions,
}: AccountSyncActivityPanelProps) {
  const rates = useTransferRates(runtimeStatus);
  const transfers = accountTransfers(runtimeStatus, rates);
  const areas = useAccountAreas();
  const problemsArea = areas.useSyncProblems(accountId, runtimeStatus?.dataRevisions.problems);
  const problems = problemsArea.data?.problems ?? [];
  const notSynced = problemsArea.data?.notSynced ?? [];
  const resolve = (paths: string[], resolution: ConflictResolution) =>
    onResolveSyncConflicts(accountId, paths, resolution);
  const openConflictCount = problems.filter((problem) => problem.isConflict && problem.resolution === null).length;
  const overview = (
    <AccountSyncOverview
      runtimeStatus={runtimeStatus}
      hasCompletedInitialSync={hasCompletedInitialSync}
      transfers={transfers}
      openConflictCount={openConflictCount}
    />
  );
  const showTabs = runtimeStatus?.authReady !== false;
  return (
    <div class="sync-panel" role="region" aria-label="Sync status">
      {!showTabs && overview}
      <AccountSyncIssues
        issueMessage={issueMessage}
        issueKind={issueKind}
        issueActions={issueActions}
        issuePath={issuePath}
        issueSecondaryPath={issueSecondaryPath}
        largeDeletePreviewPaths={largeDeletePreviewPaths}
        onOpenItemFolder={onOpenItemFolder}
        onOpenSyncRootFolder={onOpenSyncRootFolder}
        onReauthenticate={onReauthenticate}
        onRetrySync={onRetrySync}
        onConfirmLargeDelete={onConfirmLargeDelete}
        onKeepCloudFiles={onKeepCloudFiles}
        onExportLargeDeletePreview={onExportLargeDeletePreview}
      />
      {showTabs && (
        <AccountSyncActivityList
          runtimeStatus={runtimeStatus}
          summary={overview}
          rates={rates}
          transfersMoving={transfers.engineRunning}
          problemCount={problems.length}
          notSyncedCount={notSynced.length}
          notSynced={<AccountNotSynced items={notSynced} onOpenItemFolder={onOpenItemFolder} />}
          cleanup={
            <AccountCleanup
              accountId={accountId}
              revision={runtimeStatus?.dataRevisions.cleanup}
              actions={cleanupActions}
              onOpenItemFolder={onOpenItemFolder}
            />
          }
          problems={
            <AccountSyncProblems
              problems={problems}
              loadError={problemsArea.error}
              onResolve={resolve}
              onRetryFailedDownload={onRetryFailedDownload}
              onRetryAllFailedDownloads={onRetryAllFailedDownloads}
              onOpenItemFolder={onOpenItemFolder}
            />
          }
          onOpenItemFolder={onOpenItemFolder}
        />
      )}
    </div>
  );
}
