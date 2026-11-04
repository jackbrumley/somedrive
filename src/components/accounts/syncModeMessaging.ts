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
    const failedDownloads = runtimeStatus.remoteDownloadFailedTotal ?? 0;
    if (failedDownloads > 0) {
      return {
        title: "Some files could not be downloaded",
        detail: `Two-way sync starts once ${failedDownloads} failed download${failedDownloads === 1 ? " is" : "s are"} resolved.`,
        tone: "warning",
      };
    }
    if (phase === "paused") {
      return { title: "Paused", detail: "The first sync is paused. Resume when you're ready.", tone: "caution" };
    }
    if (phase === "scanning_local" || phase === "applying_local" || phase === "preparing_two_way_baseline") {
      return {
        title: "Getting ready for two-way sync",
        detail: "Checking this device before local changes start syncing to OneDrive.",
        tone: "caution",
      };
    }
    return {
      title: "Downloading your files",
      detail: "The first sync copies everything from OneDrive. Changes on this device sync once it finishes.",
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
