import type { SyncRuntimeAccountStatus } from "../../types/somedrive";
import { useTransferRates } from "../../hooks/useTransferRates";
import { AccountSyncActivityList } from "./AccountSyncActivityList";
import { AccountSyncIssues } from "./AccountSyncIssues";
import { AccountSyncOverview } from "./AccountSyncOverview";

interface AccountSyncActivityPanelProps {
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
}

/**
 * The account's sync page: a plain-language overview, anything that needs
 * the user, and the files moving now. Engine telemetry (lanes, planner
 * counters, retry queues) lives on the Debug Tools page.
 */
export function AccountSyncActivityPanel({
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
}: AccountSyncActivityPanelProps) {
  const rates = useTransferRates(runtimeStatus);
  return (
    <div class="sync-panel" role="region" aria-label="Sync status">
      <AccountSyncOverview
        runtimeStatus={runtimeStatus}
        hasCompletedInitialSync={hasCompletedInitialSync}
        downloadBytesPerSecond={rates.downloadBytesPerSecond}
        uploadBytesPerSecond={rates.uploadBytesPerSecond}
      />
      <AccountSyncIssues
        issueMessage={issueMessage}
        issueKind={issueKind}
        issueActions={issueActions}
        issuePath={issuePath}
        issueSecondaryPath={issueSecondaryPath}
        recentFailed={runtimeStatus?.recentFailed ?? []}
        largeDeletePreviewPaths={largeDeletePreviewPaths}
        onOpenItemFolder={onOpenItemFolder}
        onOpenSyncRootFolder={onOpenSyncRootFolder}
        onReauthenticate={onReauthenticate}
        onRetrySync={onRetrySync}
        onRetryFailedDownload={onRetryFailedDownload}
        onRetryAllFailedDownloads={onRetryAllFailedDownloads}
        onConfirmLargeDelete={onConfirmLargeDelete}
        onKeepCloudFiles={onKeepCloudFiles}
        onExportLargeDeletePreview={onExportLargeDeletePreview}
      />
      {runtimeStatus?.authReady !== false && (
        <AccountSyncActivityList runtimeStatus={runtimeStatus} onOpenItemFolder={onOpenItemFolder} />
      )}
    </div>
  );
}
