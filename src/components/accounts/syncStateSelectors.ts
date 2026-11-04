import type { AccountProfile, SyncRuntimeAccountStatus } from "../../types/somedrive";

function isEngineRunning(engineState: string | undefined): boolean {
  return engineState === "running";
}

export function computeEffectiveSyncState(
  _account: AccountProfile,
  runtimeStatus: SyncRuntimeAccountStatus | null
): {
  runtimeIssueCode: string | null;
  hasBlockingIssue: boolean;
  syncActive: boolean;
  syncState: "stopped" | "syncing" | "paused";
} {
  if (!runtimeStatus) {
    return {
      runtimeIssueCode: null,
      hasBlockingIssue: false,
      syncActive: false,
      syncState: "paused",
    };
  }

  const runtimeIssueCode = runtimeStatus?.issueCode ?? null;
  const phase = runtimeStatus.phase;
  const engineState = runtimeStatus.engineState ?? "paused";
  const authReady = runtimeStatus?.authReady ?? false;
  const issueSeverity = runtimeStatus?.issueSeverity ?? "none";
  const canSync = runtimeStatus?.canSync ?? false;
  const hasBlockingIssue = !authReady || issueSeverity === "blocking" || !canSync || phase === "error";
  const syncActive = isEngineRunning(engineState);
  const syncState: "stopped" | "syncing" | "paused" = hasBlockingIssue
    ? "stopped"
    : syncActive
      ? "syncing"
      : "paused";
  return {
    runtimeIssueCode,
    hasBlockingIssue,
    syncActive,
    syncState,
  };
}

/** The first sync is done once the account has completed a sync cycle. */
/**
 * Whether the account's first sync has finished. The live status carries the
 * engine's last completed cycle; the profile's copy (read at load) is used
 * only before the first status arrives.
 */
export function computeHasCompletedInitialSync(
  account: AccountProfile,
  runtimeStatus: SyncRuntimeAccountStatus | null
): boolean {
  const lastSyncAt = runtimeStatus ? runtimeStatus.lastSyncAt ?? null : account.lastSyncAt;
  return lastSyncAt !== null && lastSyncAt !== undefined;
}
