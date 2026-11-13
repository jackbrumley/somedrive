import { useAccountAreas } from "../../state/accountAreas";
import { AccountSettingsPanel } from "./AccountSettingsPanel";
import { AccountSyncActivityPanel } from "./AccountSyncActivityPanel";
import type {
  AccountProfile,
  ConflictResolution,
  CleanupActions,
  SyncRuntimeAccountStatus,
} from "../../types/somedrive";
import { computeEffectiveSyncState, computeHasCompletedInitialSync } from "./syncStateSelectors";

interface AccountDetailUnifiedPanelProps {
  account: AccountProfile;
  runtimeStatus: SyncRuntimeAccountStatus | null;
  mode: "sync" | "settings";
  onStartAuth: (accountId: string) => Promise<unknown>;
  onRename: (id: string, name: string) => Promise<void>;
  onSetSyncRoot: (id: string, path: string) => Promise<void>;
  onClearAuth: (id: string) => Promise<void>;
  onRemoveProfile: (id: string, moveLocalFolderToTrash: boolean) => Promise<void>;
  onOpenSyncRootFolder: (accountId: string) => Promise<void>;
  onOpenItemFolder: (accountId: string, relativePath: string) => Promise<void>;
  onReauthenticate: (accountId: string) => Promise<unknown>;
  onRetrySync: (accountId: string) => Promise<void>;
  onRetryFailedDownload: (accountId: string, recentItemId: string, path: string) => Promise<void>;
  onRetryAllFailedDownloads: (accountId: string) => Promise<void>;
  onConfirmLargeDelete: (accountId: string) => Promise<void>;
  onKeepCloudFiles: (accountId: string) => Promise<void>;
  onExportLargeDeletePreview: (accountId: string) => Promise<void>;
  cleanupActions: CleanupActions;
  onResolveSyncConflicts: (accountId: string, paths: string[], resolution: ConflictResolution) => Promise<void>;
}

export function AccountDetailUnifiedPanel({
  account,
  runtimeStatus,
  mode,
  onStartAuth,
  onRename,
  onSetSyncRoot,
  onClearAuth,
  onRemoveProfile,
  onOpenSyncRootFolder,
  onOpenItemFolder,
  onReauthenticate,
  onRetrySync,
  onRetryFailedDownload,
  onRetryAllFailedDownloads,
  onConfirmLargeDelete,
  onKeepCloudFiles,
  onExportLargeDeletePreview,
  cleanupActions,
  onResolveSyncConflicts,
}: AccountDetailUnifiedPanelProps) {
  const hasLargeDeleteGuardIssue = runtimeStatus?.issueCode === "large_delete_guard";
  const largeDeletePreview = useAccountAreas().useLargeDeletePreview(
    account.id,
    runtimeStatus?.dataRevisions.problems,
    mode === "sync" && hasLargeDeleteGuardIssue
  );

  const { runtimeIssueCode, hasBlockingIssue } = computeEffectiveSyncState(account, runtimeStatus);
  const runtimeAuthReady = runtimeStatus?.authReady ?? false;
  const syncIssueMessage =
    runtimeStatus?.issueMessage ??
    (!runtimeAuthReady ? null : runtimeStatus?.phaseMessage ?? "Synchronization blocked");
  const issueKind =
    runtimeStatus === null
      ? null
      : !runtimeAuthReady || runtimeIssueCode === "auth_required"
        ? "auth_required"
        : hasBlockingIssue
          ? "sync_error"
          : null;
  const issueActions = runtimeStatus?.issueActions ?? [];

  if (mode === "sync") {
    return (
      <AccountSyncActivityPanel
        accountId={account.id}
        runtimeStatus={runtimeStatus}
        hasCompletedInitialSync={computeHasCompletedInitialSync(account, runtimeStatus)}
        issueMessage={hasBlockingIssue ? syncIssueMessage : null}
        issueKind={issueKind}
        issueActions={issueActions}
        issuePath={runtimeStatus?.issuePath ?? null}
        issueSecondaryPath={runtimeStatus?.issueSecondaryPath ?? null}
        onOpenItemFolder={(relativePath) => onOpenItemFolder(account.id, relativePath)}
        onOpenSyncRootFolder={() => onOpenSyncRootFolder(account.id)}
        onReauthenticate={() => onReauthenticate(account.id)}
        onRetrySync={() => onRetrySync(account.id)}
        onRetryFailedDownload={(recentItemId, path) => onRetryFailedDownload(account.id, recentItemId, path)}
        onRetryAllFailedDownloads={() => onRetryAllFailedDownloads(account.id)}
        onConfirmLargeDelete={() => onConfirmLargeDelete(account.id)}
        onKeepCloudFiles={() => onKeepCloudFiles(account.id)}
        largeDeletePreviewPaths={largeDeletePreview.data ?? []}
        onExportLargeDeletePreview={() => onExportLargeDeletePreview(account.id)}
        cleanupActions={cleanupActions}
        onResolveSyncConflicts={onResolveSyncConflicts}
      />
    );
  }

  return (
    <AccountSettingsPanel
      account={account}
      activityRevision={runtimeStatus?.dataRevisions.activity}
      onCopyText={cleanupActions.copyText}
      onStartAuth={onStartAuth}
      onRename={onRename}
      onSetSyncRoot={onSetSyncRoot}
      onClearAuth={onClearAuth}
      onRemoveProfile={onRemoveProfile}
      onOpenSyncRootFolder={onOpenSyncRootFolder}
    />
  );
}
