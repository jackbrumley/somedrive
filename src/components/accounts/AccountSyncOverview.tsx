import {
  IconAlertTriangle,
  IconCircleCheck,
  IconClockPause,
  IconCloudDownload,
  IconHourglassHigh,
  IconInfoCircle,
} from "@tabler/icons-preact";
import type { SyncRuntimeAccountStatus } from "../../types/somedrive";
import { syncModeMessage, type SyncModeTone } from "./syncModeMessaging";
import { formatBytes, formatTransferRate, percentOf } from "./syncFormatting";

interface AccountSyncOverviewProps {
  runtimeStatus: SyncRuntimeAccountStatus | null;
  hasCompletedInitialSync: boolean;
  downloadBytesPerSecond: number;
  uploadBytesPerSecond: number;
}

interface TransferProgress {
  label: string;
  done: number;
  total: number;
  bytesLeft: number;
  bytesPerSecond: number;
}

const TONE_ICONS: Record<SyncModeTone, typeof IconInfoCircle> = {
  info: IconInfoCircle,
  success: IconCircleCheck,
  caution: IconCloudDownload,
  warning: IconAlertTriangle,
};

const IDLE_PHASES = new Set(["idle", "paused", "error"]);

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
  return { label, done, total, bytesLeft, bytesPerSecond };
}

function ProgressBlock({ progress, showSpeed }: { progress: TransferProgress; showSpeed: boolean }) {
  const percent = percentOf(progress.done, progress.total) ?? 0;
  const parts = [
    `${progress.done.toLocaleString()} of ${progress.total.toLocaleString()} files`,
    `${formatBytes(progress.bytesLeft)} left`,
  ];
  if (showSpeed) {
    parts.push(formatTransferRate(progress.bytesPerSecond));
  }
  return (
    <div class="sync-progress">
      <div class="sync-progress-header">
        <span class="sync-progress-label">{progress.label}</span>
        <span class="sync-progress-percent">{percent.toFixed(0)}%</span>
      </div>
      <div
        class="sync-progress-track"
        role="progressbar"
        aria-label={progress.label}
        aria-valuenow={Math.round(percent)}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <div class="sync-progress-fill" style={{ width: `${percent.toFixed(1)}%` }} />
      </div>
      <p class="sync-progress-meta">{parts.join(" · ")}</p>
    </div>
  );
}

export function AccountSyncOverview({
  runtimeStatus,
  hasCompletedInitialSync,
  downloadBytesPerSecond,
  uploadBytesPerSecond,
}: AccountSyncOverviewProps) {
  const message = syncModeMessage(runtimeStatus, hasCompletedInitialSync);
  const ToneIcon = TONE_ICONS[message.tone];
  const phase = runtimeStatus?.phase ?? "idle";
  const engineRunning = runtimeStatus?.engineState === "running" && !IDLE_PHASES.has(phase);

  const downloads = runtimeStatus
    ? transferProgress(
        "Downloads",
        runtimeStatus.remoteDownloadCompletedTotal ?? 0,
        runtimeStatus.remoteDownloadPlannedTotal ?? 0,
        runtimeStatus.remoteDownloadRemainingBytesTotal ?? 0,
        runtimeStatus.remoteDownloadInFlight ?? 0,
        downloadBytesPerSecond
      )
    : null;
  const uploads = runtimeStatus
    ? transferProgress(
        "Uploads",
        runtimeStatus.uploadCompletedTotal ?? 0,
        runtimeStatus.uploadPlannedTotal ?? 0,
        runtimeStatus.uploadRemainingBytesTotal ?? 0,
        runtimeStatus.uploadInFlight ?? 0,
        uploadBytesPerSecond
      )
    : null;

  const throttledRecently =
    (runtimeStatus?.remoteDownloadThrottleLastMinute ?? 0) + (runtimeStatus?.uploadThrottleLastMinute ?? 0) > 0;
  const retryWaiting = (runtimeStatus?.remoteDownloadRetryWaiting ?? 0) + (runtimeStatus?.uploadRetryWaiting ?? 0);
  const localScanCount = runtimeStatus?.localScanScannedCount ?? 0;
  const stepLabel =
    phase === "scanning_local" && localScanCount > 0
      ? `Checking this device · ${localScanCount.toLocaleString()} items`
      : runtimeStatus?.phaseMessage;

  return (
    <section class={`sync-overview sync-tone-${message.tone}`} aria-live="polite">
      <div class="sync-overview-headline">
        <span class="sync-overview-icon">
          <ToneIcon size={22} stroke={1.8} />
        </span>
        <div>
          <h3 class="sync-overview-title">{message.title}</h3>
          <p class="sync-overview-detail">{message.detail}</p>
        </div>
      </div>

      {(downloads || uploads) && (
        <div class="sync-overview-progress">
          {downloads && <ProgressBlock progress={downloads} showSpeed={engineRunning} />}
          {uploads && <ProgressBlock progress={uploads} showSpeed={engineRunning} />}
        </div>
      )}

      {(engineRunning || throttledRecently || retryWaiting > 0) && (
        <ul class="sync-overview-notes">
          {engineRunning && stepLabel && (
            <li>
              <IconHourglassHigh size={15} />
              <span>{stepLabel}</span>
            </li>
          )}
          {throttledRecently && (
            <li>
              <IconClockPause size={15} />
              <span>Microsoft is limiting requests, so SomeDrive has slowed down. It continues automatically.</span>
            </li>
          )}
          {retryWaiting > 0 && (
            <li>
              <IconClockPause size={15} />
              <span>
                {retryWaiting.toLocaleString()} file{retryWaiting === 1 ? "" : "s"} will retry automatically.
              </span>
            </li>
          )}
        </ul>
      )}
    </section>
  );
}
