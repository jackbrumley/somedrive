import {
  IconAlertTriangleFilled,
  IconBuildingBank,
  IconFolder,
  IconLoader2,
  IconPlayerPauseFilled,
  IconPlayerPlayFilled,
  IconUser,
} from "@tabler/icons-preact";
import { AccountHomeCardButton } from "./AccountHomeCardButton";
import { agentStateRequestLabel, useAgentStateRequest } from "../../hooks/useAgentStateRequest";
import type { AccountProfile, SyncRuntimeAccountStatus } from "../../types/somedrive";
import { syncModeMessage } from "./syncModeMessaging";
import { computeEffectiveSyncState, computeHasCompletedInitialSync } from "./syncStateSelectors";


/** Files that failed in the last 10 minutes (the engine sends only those). */
function recentIssueCount(runtimeStatus: SyncRuntimeAccountStatus | null): number {
  return runtimeStatus?.recentFailed.length ?? 0;
}

interface AccountCardProps {
  account: AccountProfile;
  runtimeStatus: SyncRuntimeAccountStatus | null;
  onOpenDetails: (accountId: string) => void;
  onSetAgentState: (accountId: string, state: "syncing" | "paused") => Promise<void>;
  onOpenSyncRootFolder: (accountId: string) => Promise<void>;
}

export function AccountCard({ account, runtimeStatus, onOpenDetails, onSetAgentState, onOpenSyncRootFolder }: AccountCardProps) {
  const modeMessage = syncModeMessage(runtimeStatus, computeHasCompletedInitialSync(account, runtimeStatus));
  const accountKindLabel = account.kind.charAt(0).toUpperCase() + account.kind.slice(1);
  const accountIcon = account.kind === "business" ? <IconBuildingBank size={16} /> : <IconUser size={16} />;
  const { runtimeIssueCode, hasBlockingIssue, syncActive, syncState } = computeEffectiveSyncState(account, runtimeStatus);
  const recentIssueTotal = recentIssueCount(runtimeStatus);
  const hasNonBlockingIssue = !hasBlockingIssue && (recentIssueTotal > 0 || Boolean(runtimeIssueCode));
  const nonBlockingIssueCount = recentIssueTotal + (runtimeIssueCode && !hasBlockingIssue ? 1 : 0);
  const showIssueBadge = syncState === "syncing" && hasNonBlockingIssue && nonBlockingIssueCount > 0;
  const showBlockingMarker = syncState === "stopped";
  const syncButtonClass = showBlockingMarker
    ? "account-sync-nav-btn account-sync-nav-btn-stopped"
    : "account-sync-nav-btn";
  const nextSyncState: "syncing" | "paused" = syncActive ? "paused" : "syncing";
  const { pendingState, requestAgentState } = useAgentStateRequest(account.id, onSetAgentState);
  const syncButtonTitle = showBlockingMarker
    ? "Open synchronization details"
    : pendingState !== null
      ? agentStateRequestLabel(pendingState)
      : syncState === "syncing"
      ? "Pause synchronization"
      : "Resume synchronization";
  const syncStateLabel = syncState === "stopped" ? "Stopped" : syncState === "syncing" ? "Syncing" : "Paused";

  return (
    <AccountHomeCardButton
      onClick={() => onOpenDetails(account.id)}
      ariaLabel={`Open ${account.displayName} account details`}
    >
      <div class="account-card-layout">
        <div class="account-card-left">
          <p class="account-title-line">
            <span class="account-name">{account.displayName}</span>
            <span class="account-name-sep"> - </span>
            <span class="account-email">{account.email}</span>
          </p>
          <button
            type="button"
            class="account-path-link"
            onClick={(event) => {
              event.stopPropagation();
              void onOpenSyncRootFolder(account.id);
            }}
          >
            <IconFolder size={14} /> {account.syncRoot}
          </button>
          <p class={`account-sync-mode-line account-sync-mode-line-${modeMessage.tone}`}>
            <span class="account-sync-mode-title">{modeMessage.title}</span>
            <span class="account-sync-mode-sep">: </span>
            <span class="account-sync-mode-detail">{modeMessage.detail}</span>
          </p>
        </div>
        <div class="account-card-right">
          <div class="account-sync-control-box">
            <span class="pill icon-pill account-kind-pill">{accountIcon} {accountKindLabel}</span>
            <div class="account-sync-action-wrap">
              <button
                class={syncButtonClass}
                type="button"
                aria-label={syncButtonTitle}
                title={syncButtonTitle}
                disabled={pendingState !== null}
                aria-busy={pendingState !== null}
                onClick={(event) => {
                  event.stopPropagation();
                  if (showBlockingMarker) {
                    onOpenDetails(account.id);
                    return;
                  }
                  void requestAgentState(nextSyncState);
                }}
              >
                {showBlockingMarker ? (
                  <IconAlertTriangleFilled size={24} class="sync-stopped-icon" />
                ) : pendingState !== null ? (
                  <IconLoader2 size={24} class="spin" />
                ) : syncState === "syncing" ? (
                  <IconPlayerPauseFilled size={24} />
                ) : (
                  <IconPlayerPlayFilled size={24} />
                )}
              </button>
              {showIssueBadge && (
                <span
                  class="account-sync-issue-badge"
                  aria-label={`${nonBlockingIssueCount} sync issue${nonBlockingIssueCount === 1 ? "" : "s"}`}
                >
                  {nonBlockingIssueCount > 9 ? "9+" : nonBlockingIssueCount}
                </span>
              )}
            </div>
            <span class="account-sync-state-label">{syncStateLabel}</span>
          </div>
        </div>
      </div>
    </AccountHomeCardButton>
  );
}
