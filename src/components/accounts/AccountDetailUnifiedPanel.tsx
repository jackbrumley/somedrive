import { useEffect, useState } from "preact/hooks";
import { AccountSettingsPanel } from "./AccountSettingsPanel";
import { AccountSyncActivityPanel } from "./AccountSyncActivityPanel";
import type {
  AccountProfile,
  ConflictResolution,
  SyncProblemsView,
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
  onRemoveProfile: (id: string) => Promise<void>;
  onOpenSyncRootFolder: (accountId: string) => Promise<void>;
  onOpenItemFolder: (accountId: string, relativePath: string) => Promise<void>;
  onReauthenticate: (accountId: string) => Promise<unknown>;
  onRetrySync: (accountId: string) => Promise<void>;
  onRetryFailedDownload: (accountId: string, recentItemId: string, path: string) => Promise<void>;
  onRetryAllFailedDownloads: (accountId: string) => Promise<void>;
  onConfirmLargeDelete: (accountId: string) => Promise<void>;
  onKeepCloudFiles: (accountId: string) => Promise<void>;
  onFetchLargeDeletePreview: (accountId: string) => Promise<string[]>;
  onExportLargeDeletePreview: (accountId: string) => Promise<void>;
  onFetchSyncProblems: (accountId: string) => Promise<SyncProblemsView>;
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
  onFetchLargeDeletePreview,
  onExportLargeDeletePreview,
  onFetchSyncProblems,
  onResolveSyncConflicts,
}: AccountDetailUnifiedPanelProps) {
  const [largeDeletePreviewPaths, setLargeDeletePreviewPaths] = useState<string[]>([]);

  const hasLargeDeleteGuardIssue = runtimeStatus?.issueCode === "large_delete_guard";

  useEffect(() => {
    if (mode !== "sync" || !hasLargeDeleteGuardIssue) {
      setLargeDeletePreviewPaths([]);
      return;
    }
    let active = true;
    void onFetchLargeDeletePreview(account.id).then((paths) => {
      if (active) {
        setLargeDeletePreviewPaths(paths);
      }
    });
    return () => {
      active = false;
    };
  }, [account.id, hasLargeDeleteGuardIssue, mode, onFetchLargeDeletePreview]);

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
        largeDeletePreviewPaths={largeDeletePreviewPaths}
        onExportLargeDeletePreview={() => onExportLargeDeletePreview(account.id)}
        onFetchSyncProblems={onFetchSyncProblems}
        onResolveSyncConflicts={onResolveSyncConflicts}
      />
    );
  }

  return (
    <AccountSettingsPanel
      account={account}
      onStartAuth={onStartAuth}
      onRename={onRename}
      onSetSyncRoot={onSetSyncRoot}
      onClearAuth={onClearAuth}
      onRemoveProfile={onRemoveProfile}
      onOpenSyncRootFolder={onOpenSyncRootFolder}
    />
  );
}
