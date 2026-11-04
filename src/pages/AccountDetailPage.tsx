import {
  IconAdjustments,
  IconAlertTriangleFilled,
  IconBuildingBank,
  IconLoader2,
  IconPlayerPauseFilled,
  IconPlayerPlayFilled,
  IconUser,
} from "@tabler/icons-preact";
import { memo } from "preact/compat";
import { agentStateRequestLabel, useAgentStateRequest } from "../hooks/useAgentStateRequest";
import { PageHeader } from "../components/shell/PageHeader";
import { AccountDetailUnifiedPanel } from "../components/accounts/AccountDetailUnifiedPanel";
import type { AccountProfile, ConflictResolution, SyncProblem, SyncRuntimeAccountStatus } from "../types/somedrive";
import { computeEffectiveSyncState } from "../components/accounts/syncStateSelectors";

interface AccountDetailPageProps {
  account: AccountProfile | null;
  runtimeStatus: SyncRuntimeAccountStatus | null;
  view: "sync" | "settings";
  onBack: () => void;
  onOpenSettings: (accountId: string) => void;
  onOpenSync: (accountId: string) => void;
  onSetAgentState: (accountId: string, state: "syncing" | "paused") => Promise<void>;
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
  onFetchSyncProblems: (accountId: string) => Promise<SyncProblem[]>;
  onResolveSyncConflicts: (accountId: string, paths: string[], resolution: ConflictResolution) => Promise<void>;
}

interface AccountDetailHeaderProps {
  account: AccountProfile;
  runtimeStatus: SyncRuntimeAccountStatus | null;
  view: "sync" | "settings";
  onBack: () => void;
  onOpenSettings: (accountId: string) => void;
  onOpenSync: (accountId: string) => void;
  onSetAgentState: (accountId: string, state: "syncing" | "paused") => Promise<void>;
}

const AccountDetailHeader = memo(function AccountDetailHeader({
  account,
  runtimeStatus,
  view,
  onBack,
  onOpenSettings,
  onOpenSync,
  onSetAgentState,
}: AccountDetailHeaderProps) {
  const { syncState } = computeEffectiveSyncState(account, runtimeStatus);
  const { pendingState, requestAgentState } = useAgentStateRequest(account.id, onSetAgentState);
  const accountKindLabel = account.kind === "business" ? "Business" : "Personal";
  const accountKindIcon = account.kind === "business" ? <IconBuildingBank size={14} /> : <IconUser size={14} />;
  const isSyncView = view === "sync";

  // The same actions show in both views so the header never shifts.
  const actions = (
    <>
      <span class="pill icon-pill account-kind-pill">
        {accountKindIcon} {accountKindLabel}
      </span>
      <button
        type="button"
        class={isSyncView ? "ghost header-icon-btn" : "ghost header-icon-btn is-active"}
        aria-label={isSyncView ? "Open account settings" : "Close account settings"}
        aria-pressed={!isSyncView}
        title={isSyncView ? "Account settings" : "Back to sync status"}
        onClick={() => (isSyncView ? onOpenSettings(account.id) : onOpenSync(account.id))}
      >
        <IconAdjustments size={16} stroke={2} />
      </button>
      {syncState === "stopped" ? (
        <span class="header-sync-state is-stopped" title="Sync is stopped. Details are on the sync page.">
          <IconAlertTriangleFilled size={14} />
          Stopped
        </span>
      ) : (
        <button
          type="button"
          class="ghost header-sync-btn"
          disabled={pendingState !== null}
          aria-busy={pendingState !== null}
          onClick={() => void requestAgentState(syncState === "syncing" ? "paused" : "syncing")}
        >
          {pendingState !== null ? (
            <>
              <IconLoader2 size={14} class="spin" />
              {agentStateRequestLabel(pendingState)}
            </>
          ) : (
            <>
              {syncState === "syncing" ? <IconPlayerPauseFilled size={14} /> : <IconPlayerPlayFilled size={14} />}
              {syncState === "syncing" ? "Pause" : "Resume"}
            </>
          )}
        </button>
      )}
    </>
  );

  return (
    <PageHeader
      title={account.displayName}
      back={
        isSyncView
          ? { href: "#/accounts", label: "Back to accounts", onBack }
          : {
              href: `#/accounts/${encodeURIComponent(account.id)}`,
              label: "Back to sync status",
              onBack: () => onOpenSync(account.id),
            }
      }
      actions={actions}
    />
  );
});

export function AccountDetailPage({
  account,
  runtimeStatus,
  view,
  onBack,
  onOpenSettings,
  onOpenSync,
  onSetAgentState,
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
}: AccountDetailPageProps) {
  if (!account) {
    return (
      <section class="page account-detail-page">
        <PageHeader title="Account not found" back={{ href: "#/accounts", label: "Back to accounts", onBack }} />
        <div class="page-scroll">
          <article class="card">
            <p>This account does not exist anymore. Return to the account list.</p>
            <button onClick={onBack}>Back to Accounts</button>
          </article>
        </div>
      </section>
    );
  }

  return (
    <section class="page account-detail-page">
      <AccountDetailHeader
        account={account}
        runtimeStatus={runtimeStatus}
        view={view}
        onBack={onBack}
        onOpenSettings={onOpenSettings}
        onOpenSync={onOpenSync}
        onSetAgentState={onSetAgentState}
      />
      <div class="page-scroll">
        <AccountDetailUnifiedPanel
          account={account}
          runtimeStatus={runtimeStatus}
          mode={view}
          onStartAuth={onStartAuth}
          onRename={onRename}
          onSetSyncRoot={onSetSyncRoot}
          onClearAuth={onClearAuth}
          onRemoveProfile={onRemoveProfile}
          onOpenSyncRootFolder={onOpenSyncRootFolder}
          onOpenItemFolder={onOpenItemFolder}
          onReauthenticate={onReauthenticate}
          onRetrySync={onRetrySync}
          onRetryFailedDownload={onRetryFailedDownload}
          onRetryAllFailedDownloads={onRetryAllFailedDownloads}
          onConfirmLargeDelete={onConfirmLargeDelete}
          onKeepCloudFiles={onKeepCloudFiles}
          onFetchLargeDeletePreview={onFetchLargeDeletePreview}
          onExportLargeDeletePreview={onExportLargeDeletePreview}
          onFetchSyncProblems={onFetchSyncProblems}
          onResolveSyncConflicts={onResolveSyncConflicts}
        />
      </div>
    </section>
  );
}
