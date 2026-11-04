import type { SyncRuntimeAccountStatus } from "../../types/somedrive";

export type SyncModeTone = "info" | "success" | "warning" | "caution";

export interface SyncModeMessage {
  title: string;
  detail: string;
  tone: SyncModeTone;
}

function isDeletionGuardIssue(runtimeStatus: SyncRuntimeAccountStatus): boolean {
  if (runtimeStatus.issueCode === "large_delete_guard") {
    return true;
  }
  return runtimeStatus.issueActions.includes("confirm_large_delete") || runtimeStatus.issueActions.includes("keep_cloud_files");
}

function hasActiveTransfers(runtimeStatus: SyncRuntimeAccountStatus): boolean {
  return (runtimeStatus.remoteDownloadInFlight ?? 0) > 0 || (runtimeStatus.uploadInFlight ?? 0) > 0;
}

/** The one-line, plain-language headline for an account's sync state. */
export function syncModeMessage(
  runtimeStatus: SyncRuntimeAccountStatus | null,
  hasCompletedInitialSync: boolean
): SyncModeMessage {
  if (!runtimeStatus) {
    return { title: "Connecting", detail: "Waiting for the first status update from the sync engine.", tone: "info" };
  }

  if (runtimeStatus.authReady === false) {
    return {
      title: "Sign-in needed",
      detail:
        runtimeStatus.issueCode === "auth_required" && runtimeStatus.issueMessage
          ? runtimeStatus.issueMessage
          : "Sign in to Microsoft to start syncing this account.",
      tone: "warning",
    };
  }

  if (isDeletionGuardIssue(runtimeStatus)) {
    return {
      title: "Large deletion detected",
      detail: "Cloud deletion is paused until you review these changes.",
      tone: "warning",
    };
  }

  const phase = runtimeStatus.phase;
  if (phase === "error") {
    return {
      title: "Sync stopped",
      detail: runtimeStatus.issueMessage ?? "A problem stopped sync. Details are below.",
      tone: "warning",
    };
  }

  if (!hasCompletedInitialSync) {
    if (phase === "paused") {
      return { title: "Paused", detail: "The first sync is paused. Resume when you're ready.", tone: "caution" };
    }
    return {
      title: "First sync in progress",
      detail: "Bringing this device and OneDrive into step. Files already here are compared, never overwritten.",
      tone: "caution",
    };
  }

  if (phase === "paused") {
    return { title: "Paused", detail: "Resume to keep this device and OneDrive in step.", tone: "info" };
  }

  if (hasActiveTransfers(runtimeStatus)) {
    return { title: "Syncing", detail: "Changes on this device and in OneDrive are being synced.", tone: "success" };
  }

  return { title: "Up to date", detail: "Changes on this device and in OneDrive stay in step.", tone: "success" };
}
