import type { SyncRuntimeAccountStatus } from "../../types/somedrive";
import { formatBytes, formatTransferRate, percentOf } from "./syncFormatting";

/** Phases in which nothing is moving. */
export const IDLE_PHASES = new Set(["idle", "paused", "error"]);

export interface TransferProgress {
  label: string;
  done: number;
  total: number;
  unit: string;
  /** Null for counted work that moves no data (deletes, file checks). */
  bytesLeft: number | null;
  bytesPerSecond: number;
}

/** The account's work in progress, for the Summary tab and the header. */
export interface AccountTransfers {
  downloads: TransferProgress | null;
  uploads: TransferProgress | null;
  /** The engine's own counted step, e.g. "Deleting from OneDrive". */
  counted: TransferProgress | null;
  /** Transfers are moving now (not paused, idle or stopped). */
  engineRunning: boolean;
  /** Microsoft refused a request in the last minute. */
  throttledRecently: boolean;
}

interface TransferRates {
  downloadBytesPerSecond: number;
  uploadBytesPerSecond: number;
}

function transferProgress(
  label: string,
  done: number,
  total: number,
  bytesLeft: number,
  inFlight: number,
  bytesPerSecond: number
): TransferProgress | null {
  if (total <= 0 || (done >= total && inFlight === 0)) {
    return null;
  }
  return { label, done, total, unit: "files", bytesLeft, bytesPerSecond };
}

function countedProgress(runtimeStatus: SyncRuntimeAccountStatus): TransferProgress | null {
  const activity = runtimeStatus.currentActivity;
  if (runtimeStatus.phase === "paused" || activity?.progressMode !== "determinate" || !activity.detail) {
    return null;
  }
  const total = activity.total ?? 0;
  if (total <= 0) {
    return null;
  }
  return {
    label: activity.detail,
    done: activity.current ?? 0,
    total,
    unit: activity.unit ?? "items",
    bytesLeft: null,
    bytesPerSecond: 0,
  };
}

export function accountTransfers(
  runtimeStatus: SyncRuntimeAccountStatus | null,
  rates: TransferRates
): AccountTransfers {
  if (!runtimeStatus) {
    return { downloads: null, uploads: null, counted: null, engineRunning: false, throttledRecently: false };
  }
  return {
    downloads: transferProgress(
      "Downloads",
      runtimeStatus.remoteDownloadCompletedTotal ?? 0,
      runtimeStatus.remoteDownloadPlannedTotal ?? 0,
      runtimeStatus.remoteDownloadRemainingBytesTotal ?? 0,
      runtimeStatus.remoteDownloadInFlight ?? 0,
      rates.downloadBytesPerSecond
    ),
    uploads: transferProgress(
      "Uploads",
      runtimeStatus.uploadCompletedTotal ?? 0,
      runtimeStatus.uploadPlannedTotal ?? 0,
      runtimeStatus.uploadRemainingBytesTotal ?? 0,
      runtimeStatus.uploadInFlight ?? 0,
      rates.uploadBytesPerSecond
    ),
    counted: countedProgress(runtimeStatus),
    engineRunning: runtimeStatus.engineState === "running" && !IDLE_PHASES.has(runtimeStatus.phase),
    throttledRecently:
      (runtimeStatus.remoteDownloadThrottleLastMinute ?? 0) + (runtimeStatus.uploadThrottleLastMinute ?? 0) > 0,
  };
}

/** The one-line progress in the page header: a bar's percent and its text. */
export interface HeaderProgress {
  percent: number;
  summary: string;
  /** The fuller wording, for the tooltip and screen readers. */
  label: string;
}

/**
 * Downloads and uploads together as one bar (files done of files planned),
 * with the data left and the speed; a counted step (deletes, checks) when
 * nothing transfers. Null when there is nothing to show.
 */
export function headerProgress(transfers: AccountTransfers): HeaderProgress | null {
  const moving = [transfers.downloads, transfers.uploads].filter(
    (progress): progress is TransferProgress => progress !== null
  );
  if (moving.length === 0) {
    const counted = transfers.counted;
    if (!counted) {
      return null;
    }
    const percent = percentOf(counted.done, counted.total) ?? 0;
    return {
      percent,
      summary: `${counted.label} · ${percent.toFixed(0)}%`,
      label: `${counted.label}: ${counted.done.toLocaleString()} of ${counted.total.toLocaleString()} ${counted.unit}`,
    };
  }
  const done = moving.reduce((sum, progress) => sum + progress.done, 0);
  const total = moving.reduce((sum, progress) => sum + progress.total, 0);
  const bytesLeft = moving.reduce((sum, progress) => sum + (progress.bytesLeft ?? 0), 0);
  const percent = percentOf(done, total) ?? 0;
  const parts = [`${percent.toFixed(0)}%`, `${formatBytes(bytesLeft)} left`];
  if (transfers.throttledRecently) {
    parts.push("slowed by Microsoft");
  } else if (transfers.engineRunning) {
    parts.push(
      moving.length === 2
        ? `↓ ${formatTransferRate(moving[0].bytesPerSecond)} ↑ ${formatTransferRate(moving[1].bytesPerSecond)}`
        : formatTransferRate(moving[0].bytesPerSecond)
    );
  } else {
    parts.push("paused");
  }
  const direction = moving.length === 2 ? "Syncing" : moving[0].label === "Downloads" ? "Downloading" : "Uploading";
  return {
    percent,
    summary: parts.join(" · "),
    label: `${direction}: ${done.toLocaleString()} of ${total.toLocaleString()} files, ${parts.slice(1).join(", ")}`,
  };
}
