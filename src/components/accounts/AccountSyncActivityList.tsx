import { IconCircleCheckFilled, IconDownload, IconUpload } from "@tabler/icons-preact";
import type { ComponentChildren } from "preact";
import { useRef, useState } from "preact/hooks";
import { useElementHeight } from "../../hooks/useElementHeight";
import type { SyncRuntimeAccountStatus } from "../../types/somedrive";
import { VirtualizedList } from "../ui/VirtualizedList";
import { AccountSyncDetails } from "./AccountSyncDetails";
import { FileTypeIcon } from "./FileTypeIcon";
import { formatBytes, formatClockTime, percentOf, splitSyncPath } from "./syncFormatting";

const ROW_HEIGHT_PX = 52;
const MIN_VISIBLE_ROWS = 4;
const RECENT_LIMIT = 64;

type ActivityTab = "now" | "recent" | "problems" | "details";

interface ActivityRow {
  id: string;
  direction: string;
  path: string;
  sizeText: string;
  progressPercent: number | null;
  completed: boolean;
}

interface AccountSyncActivityListProps {
  runtimeStatus: SyncRuntimeAccountStatus | null;
  problemCount: number;
  problems: ComponentChildren;
  onOpenItemFolder: (relativePath: string) => Promise<void>;
}

function activeRows(runtimeStatus: SyncRuntimeAccountStatus): ActivityRow[] {
  if (runtimeStatus.phase === "paused") {
    return [];
  }
  return runtimeStatus.inProgress
    .filter((transfer) => transfer.state !== "queued")
    .sort((left, right) => Date.parse(left.startedAt) - Date.parse(right.startedAt) || left.path.localeCompare(right.path))
    .map((transfer) => {
      const progressPercent = percentOf(transfer.bytesDone, transfer.bytesTotal);
      return {
        id: transfer.id,
        direction: transfer.direction,
        path: transfer.path,
        sizeText:
          transfer.bytesTotal === null
            ? "Size unknown"
            : transfer.bytesDone > 0
              ? `${formatBytes(transfer.bytesDone)} of ${formatBytes(transfer.bytesTotal)}`
              : formatBytes(transfer.bytesTotal),
        progressPercent,
        completed: false,
      };
    });
}

function recentRows(runtimeStatus: SyncRuntimeAccountStatus): ActivityRow[] {
  return [...runtimeStatus.recentCompleted]
    .sort((left, right) => Date.parse(right.finishedAt) - Date.parse(left.finishedAt))
    .slice(0, RECENT_LIMIT)
    .map((item) => ({
      id: item.id,
      direction: item.direction,
      path: item.path,
      sizeText: `${formatBytes(item.bytesTotal)} · ${formatClockTime(item.finishedAt)}`,
      progressPercent: null,
      completed: true,
    }));
}

function ActivityRowView({ row, onOpen }: { row: ActivityRow; onOpen: (path: string) => Promise<void> }) {
  const { name, folder } = splitSyncPath(row.path);
  const DirectionIcon = row.direction.toLowerCase() === "upload" ? IconUpload : IconDownload;
  return (
    <button type="button" class="sync-file-open sync-activity-row" title={row.path} onClick={() => void onOpen(row.path)}>
      <span class="sync-file-icon">
        <FileTypeIcon path={row.path} size={20} />
      </span>
      <span class="sync-file-text">
        <span class="sync-file-name">{name}</span>
        <span class="sync-file-folder">{folder || "OneDrive"}</span>
      </span>
      <span class="sync-activity-size">
        {row.sizeText}
        {row.progressPercent !== null && (
          <span class="sync-activity-mini-track">
            <span class="sync-activity-mini-fill" style={{ width: `${row.progressPercent.toFixed(0)}%` }} />
          </span>
        )}
      </span>
      <span class={row.completed ? "sync-activity-state is-done" : "sync-activity-state"}>
        {row.completed ? <IconCircleCheckFilled size={17} /> : <DirectionIcon size={17} />}
      </span>
    </button>
  );
}

export function AccountSyncActivityList({
  runtimeStatus,
  problemCount,
  problems,
  onOpenItemFolder,
}: AccountSyncActivityListProps) {
  const [selectedTab, setSelectedTab] = useState<ActivityTab>("now");
  const listBodyRef = useRef<HTMLDivElement>(null);
  const listBodyHeight = useElementHeight(listBodyRef);
  const nowRows = runtimeStatus ? activeRows(runtimeStatus) : [];
  const doneRows = runtimeStatus ? recentRows(runtimeStatus) : [];
  const tab: ActivityTab = selectedTab === "now" && nowRows.length === 0 && doneRows.length > 0 ? "recent" : selectedTab;
  const rows = tab === "recent" ? doneRows : nowRows;

  return (
    <section class="sync-activity">
      <div class="sync-activity-tabs" role="tablist" aria-label="Sync activity and details">
        <button
          type="button"
          role="tab"
          aria-selected={tab === "now"}
          class={tab === "now" ? "sync-activity-tab is-active" : "sync-activity-tab"}
          onClick={() => setSelectedTab("now")}
        >
          In progress <span class="sync-activity-count">{nowRows.length}</span>
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={tab === "recent"}
          class={tab === "recent" ? "sync-activity-tab is-active" : "sync-activity-tab"}
          onClick={() => setSelectedTab("recent")}
        >
          Recently synced <span class="sync-activity-count">{doneRows.length}</span>
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={tab === "problems"}
          class={tab === "problems" ? "sync-activity-tab is-active" : "sync-activity-tab"}
          onClick={() => setSelectedTab("problems")}
        >
          Sync problems{" "}
          <span class={problemCount > 0 ? "sync-activity-count is-attention" : "sync-activity-count"}>{problemCount}</span>
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={tab === "details"}
          class={tab === "details" ? "sync-activity-tab is-active" : "sync-activity-tab"}
          onClick={() => setSelectedTab("details")}
        >
          Details
        </button>
      </div>
      <div class="sync-activity-body" ref={listBodyRef}>
        {tab === "problems" ? (
          <div class="sync-tab-scroll">{problems}</div>
        ) : tab === "details" ? (
          runtimeStatus ? (
            <AccountSyncDetails runtimeStatus={runtimeStatus} />
          ) : (
            <p class="sync-activity-empty">No status has been received for this account yet.</p>
          )
        ) : rows.length === 0 ? (
          <p class="sync-activity-empty">{tab === "now" ? "Nothing transferring right now." : "No files synced yet."}</p>
        ) : (
          <VirtualizedList
            items={rows}
            rowHeight={ROW_HEIGHT_PX}
            maxHeight={Math.max(listBodyHeight, ROW_HEIGHT_PX * MIN_VISIBLE_ROWS)}
            className="sync-activity-list"
            keyExtractor={(row) => row.id}
            renderItem={(row) => <ActivityRowView row={row} onOpen={onOpenItemFolder} />}
          />
        )}
      </div>
    </section>
  );
}
