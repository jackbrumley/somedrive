import {
  IconAlertTriangle,
  IconArrowsDiff,
  IconCircleCheck,
  IconClockCheck,
  IconClockPause,
  IconCloudDownload,
  IconHourglassHigh,
  IconInfoCircle,
  IconRadar,
} from "@tabler/icons-preact";
import type { SyncRuntimeAccountStatus } from "../../types/somedrive";
import { syncModeMessage, type SyncModeTone } from "./syncModeMessaging";
import { formatBytes, formatTransferRate, percentOf } from "./syncFormatting";
import type { AccountTransfers, TransferProgress } from "./transferProgress";

interface AccountSyncOverviewProps {
  runtimeStatus: SyncRuntimeAccountStatus | null;
  hasCompletedInitialSync: boolean;
  transfers: AccountTransfers;
  openConflictCount: number;
}

const TONE_ICONS: Record<SyncModeTone, typeof IconInfoCircle> = {
  info: IconInfoCircle,
  success: IconCircleCheck,
  caution: IconCloudDownload,
  warning: IconAlertTriangle,
};

function ProgressBlock({ progress, showSpeed }: { progress: TransferProgress; showSpeed: boolean }) {
  const percent = percentOf(progress.done, progress.total) ?? 0;
  const parts = [`${progress.done.toLocaleString()} of ${progress.total.toLocaleString()} ${progress.unit}`];
  if (progress.bytesLeft !== null) {
    parts.push(`${formatBytes(progress.bytesLeft)} left`);
    if (showSpeed) {
      parts.push(formatTransferRate(progress.bytesPerSecond));
    }
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
  transfers,
  openConflictCount,
}: AccountSyncOverviewProps) {
  const message = syncModeMessage(runtimeStatus, hasCompletedInitialSync);
  const ToneIcon = TONE_ICONS[message.tone];
  const phase = runtimeStatus?.phase ?? "idle";
  const { downloads, uploads, counted, engineRunning, throttledRecently } = transfers;

  // With a counted bar below, the headline need not repeat its numbers.
  const headlineDetail = counted ? "Changes on this device and in OneDrive are being synced." : message.detail;

  const retryWaiting = (runtimeStatus?.remoteDownloadRetryWaiting ?? 0) + (runtimeStatus?.uploadRetryWaiting ?? 0);
  const localScanCount = runtimeStatus?.localScanScannedCount ?? 0;
  const stepLabel =
    phase === "scanning_local" && localScanCount > 0
      ? `Checking this device · ${localScanCount.toLocaleString()} items`
      : runtimeStatus?.phaseMessage;
  // The step line stays while the account is syncing, idle or busy, so the
  // check every few seconds changes its text instead of pushing the page
  // below down and back up.
  const showStepLine = runtimeStatus?.engineState === "running" && phase !== "paused" && phase !== "error";
  const notWatched = runtimeStatus?.localChangesWatched === false;

  return (
    <section class={`sync-overview sync-tone-${message.tone}`} aria-live="polite">
      <div class="sync-overview-headline">
        <span class="sync-overview-icon">
          <ToneIcon size={22} stroke={1.8} />
        </span>
        <div>
          <h3 class="sync-overview-title">{message.title}</h3>
          <p class="sync-overview-detail">{headlineDetail}</p>
        </div>
      </div>

      {(counted || downloads || uploads) && (
        <div class="sync-overview-progress">
          {counted && <ProgressBlock progress={counted} showSpeed={false} />}
          {downloads && <ProgressBlock progress={downloads} showSpeed={engineRunning} />}
          {uploads && <ProgressBlock progress={uploads} showSpeed={engineRunning} />}
        </div>
      )}

      {(showStepLine || notWatched || throttledRecently || retryWaiting > 0 || openConflictCount > 0) && (
        <ul class="sync-overview-notes">
          {openConflictCount > 0 && (
            <li class="sync-overview-note-attention">
              <IconArrowsDiff size={15} />
              <span>
                {openConflictCount.toLocaleString()} file{openConflictCount === 1 ? " needs" : "s need"} your decision in
                Sync problems. Nothing has been changed or duplicated.
              </span>
            </li>
          )}
          {showStepLine && (
            <li class="sync-overview-step">
              {engineRunning ? <IconHourglassHigh size={15} /> : <IconClockCheck size={15} />}
              <span>{stepLabel || "Waiting for the next check"}</span>
            </li>
          )}
          {notWatched && (
            <li>
              <IconRadar size={15} />
              <span>
                This folder can't be watched for changes, so SomeDrive checks it every 15 seconds instead. Saved files
                may take a little longer to start syncing.
              </span>
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
