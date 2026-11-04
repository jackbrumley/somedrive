import type { SyncRuntimeAccountStatus } from "../../types/somedrive";
import { formatBytes, formatClockTime } from "./syncFormatting";

const RETRY_PREVIEW_LIMIT = 50;

interface MetricGroup {
  title: string;
  metrics: Array<[label: string, value: string]>;
}

function count(value: number | undefined): string {
  return (value ?? 0).toLocaleString();
}

function metricGroups(status: SyncRuntimeAccountStatus): MetricGroup[] {
  return [
    {
      title: "Lifecycle",
      metrics: [
        ["Engine state", status.engineState ?? "unknown"],
        ["Phase", status.phase],
        ["Phase message", status.phaseMessage || "—"],
        ["Activity stage", status.currentActivity?.stage ?? "—"],
        ["Activity detail", status.currentActivity?.detail ?? "—"],
        ["Auth ready", String(status.authReady ?? false)],
        ["Can sync", String(status.canSync ?? false)],
        ["Issue", status.issueCode ?? "none"],
        ["Remote scan complete", String(status.remoteScanComplete ?? false)],
        ["Two-way ready", String(status.twoWayReady ?? false)],
        ["Updated", formatClockTime(status.updatedAt)],
      ],
    },
    {
      title: "Discovery and planner",
      metrics: [
        ["Cloud files discovered", count(status.plannerCloudDiscoveredTotal)],
        ["Local files discovered", count(status.plannerLocalDiscoveredTotal)],
        ["Need download", count(status.plannerNeedDownloadTotal)],
        ["Need upload", count(status.plannerNeedUploadTotal)],
        ["Need delete remote", count(status.plannerNeedDeleteRemoteTotal)],
        ["Need delete local", count(status.plannerNeedDeleteLocalTotal)],
        ["Need conflict handling", count(status.plannerConflictTotal)],
        ["No action", count(status.plannerNoneTotal)],
      ],
    },
    {
      title: "Downloads",
      metrics: [
        ["Planned", count(status.remoteDownloadPlannedTotal)],
        ["Completed", count(status.remoteDownloadCompletedTotal)],
        ["In flight", count(status.remoteDownloadInFlight)],
        ["Retry waiting", count(status.remoteDownloadRetryWaiting)],
        ["Failed", count(status.remoteDownloadFailedTotal)],
        ["Downloaded size", formatBytes(status.remoteDownloadCompletedBytesTotal ?? 0)],
        ["Remaining size", formatBytes(status.remoteDownloadRemainingBytesTotal ?? 0)],
        ["Rate limits (last minute)", count(status.remoteDownloadThrottleLastMinute)],
        ["Rate limits (total)", count(status.remoteDownloadThrottleTotal)],
      ],
    },
    {
      title: "Uploads",
      metrics: [
        ["Planned", count(status.uploadPlannedTotal)],
        ["Completed", count(status.uploadCompletedTotal)],
        ["In flight", count(status.uploadInFlight)],
        ["Retry waiting", count(status.uploadRetryWaiting)],
        ["Failed", count(status.uploadFailedTotal)],
        ["Uploaded size", formatBytes(status.uploadCompletedBytesTotal ?? 0)],
        ["Remaining size", formatBytes(status.uploadRemainingBytesTotal ?? 0)],
        ["Rate limits (last minute)", count(status.uploadThrottleLastMinute)],
        ["Rate limits (total)", count(status.uploadThrottleTotal)],
      ],
    },
  ];
}

interface AccountSyncDetailsProps {
  runtimeStatus: SyncRuntimeAccountStatus;
}

/** The sync engine's raw state for one account: lifecycle, planner and transfer counters. */
export function AccountSyncDetails({ runtimeStatus }: AccountSyncDetailsProps) {
  return (
    <div class="sync-details">
      {runtimeStatus.issueMessage && <p class="diagnostics-issue">{runtimeStatus.issueMessage}</p>}
      {runtimeStatus.consistency?.ok === false && (
        <div class="diagnostics-group">
          <p class="diagnostics-group-title">Consistency violations</p>
          <ul class="diagnostics-list">
            {runtimeStatus.consistency.violations.map((violation) => (
              <li key={violation}>{violation}</li>
            ))}
          </ul>
        </div>
      )}
      <div class="diagnostics-groups">
        {metricGroups(runtimeStatus).map((group) => (
          <section key={group.title} class="diagnostics-group">
            <p class="diagnostics-group-title">{group.title}</p>
            <dl class="diagnostics-metrics">
              {group.metrics.map(([label, value]) => (
                <div key={label} class="diagnostics-metric">
                  <dt>{label}</dt>
                  <dd>{value}</dd>
                </div>
              ))}
            </dl>
          </section>
        ))}
      </div>
      {runtimeStatus.recentRetryWaiting.length > 0 && (
        <details class="diagnostics-group">
          <summary class="diagnostics-group-title">Waiting to retry ({runtimeStatus.recentRetryWaiting.length})</summary>
          <ul class="diagnostics-list">
            {runtimeStatus.recentRetryWaiting.slice(0, RETRY_PREVIEW_LIMIT).map((item) => (
              <li key={item.id}>
                <span class="diagnostics-path">{item.path}</span>
                <span class="diagnostics-meta">
                  {item.error ?? "Retry queued"} · after {formatClockTime(item.finishedAt)}
                </span>
              </li>
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}
