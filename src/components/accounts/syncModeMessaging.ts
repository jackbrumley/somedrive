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

/**
 * The engine's own counted progress, when it reports one, e.g.
 * "Checking files against OneDrive · 12,345 of 15,600 files (79%)".
 */
export function countedActivityLine(runtimeStatus: SyncRuntimeAccountStatus): string | null {
  const activity = runtimeStatus.currentActivity;
  if (!activity || activity.progressMode !== "determinate" || !activity.detail) {
    return null;
  }
  const current = activity.current ?? 0;
  const total = activity.total ?? 0;
  if (total <= 0) {
    return activity.detail;
  }
  const percent = Math.floor((current / total) * 100);
  const unit = activity.unit ? ` ${activity.unit}` : "";
  return `${activity.detail} · ${current.toLocaleString()} of ${total.toLocaleString()}${unit} (${percent}%)`;
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

  const counted = phase === "paused" ? null : countedActivityLine(runtimeStatus);

  if (!hasCompletedInitialSync) {
    if (phase === "paused") {
      return { title: "Paused", detail: "The first sync is paused. Resume when you're ready.", tone: "caution" };
    }
    return {
      title: "First sync in progress",
      detail:
        counted ??
        "Bringing this device and OneDrive into step. Files already here are compared, never overwritten.",
      tone: "caution",
    };
  }

  if (phase === "paused") {
    return { title: "Paused", detail: "Resume to keep this device and OneDrive in step.", tone: "info" };
  }

  if (counted) {
    return { title: "Syncing", detail: counted, tone: "success" };
  }

  if (hasActiveTransfers(runtimeStatus)) {
    return { title: "Syncing", detail: "Changes on this device and in OneDrive are being synced.", tone: "success" };
  }

  return { title: "Up to date", detail: "Changes on this device and in OneDrive stay in step.", tone: "success" };
}

/**
 * The account's state in a few words, for the page header when nothing is
 * transferring (with transfers, the header shows their progress instead).
 * While the engine works through a step, the step is named, e.g.
 * "First sync · Fetching remote files". After the first sync the periodic
 * checks stay "Up to date", so the text does not flicker every few seconds.
 */
export function headerStatusText(
  runtimeStatus: SyncRuntimeAccountStatus | null,
  message: SyncModeMessage,
  engineRunning: boolean
): string {
  const step = runtimeStatus?.phaseMessage?.trim();
  if (engineRunning && step) {
    if (message.title === "First sync in progress") {
      return `First sync · ${step}`;
    }
    if (message.title === "Syncing") {
      return `Syncing · ${step}`;
    }
  }
  return message.title;
}
