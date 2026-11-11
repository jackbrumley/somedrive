import { IconAlertTriangle, IconCircleCheckFilled, IconDownload, IconTrash, IconUpload } from "@tabler/icons-preact";
import type { ComponentChildren } from "preact";
import { useRef, useState } from "preact/hooks";
import { useElementHeight } from "../../hooks/useElementHeight";
import type { SyncRuntimeAccountStatus, SyncRuntimeRecentItem } from "../../types/somedrive";
import { VirtualizedList } from "../ui/VirtualizedList";
import { AccountSyncDetails } from "./AccountSyncDetails";
import { FileTypeIcon } from "./FileTypeIcon";
import { formatBytes, formatClockTime, percentOf, splitSyncPath } from "./syncFormatting";

const ROW_HEIGHT_PX = 52;
const MIN_VISIBLE_ROWS = 4;
/** Finished rows shown at most (the engine sends the last 10 minutes). */
const FINISHED_LIMIT = 200;

type ActivityTab = "summary" | "activity" | "problems" | "notSynced" | "cleanup" | "details";

/** What a feed row is: still running, or how it finished. */
type RowKind = "running" | "done" | "deleted" | "failed";

interface ActivityRow {
  id: string;
  kind: RowKind;
  direction: string;
  path: string;
  /** What happened or is happening, short: "Downloaded · 1.2 MB". */
  detail: string;
  /** The second line when there is no progress bar: the time. */
  when: string;
  /** The full explanation, shown on hover. */
  explanation: string;
  progressPercent: number | null;
}

interface AccountSyncActivityListProps {
  runtimeStatus: SyncRuntimeAccountStatus | null;
  /** The plain-language overview with the full progress bars. */
  summary: ComponentChildren;
  /** Transfers are moving: the page opens on Activity, otherwise Summary. */
  transfersMoving: boolean;
  problemCount: number;
  problems: ComponentChildren;
  notSyncedCount: number;
  notSynced: ComponentChildren;
  cleanup: ComponentChildren;
  onOpenItemFolder: (relativePath: string) => Promise<void>;
}

/** What a finished delete did (mirrors `sync_history_store`): a short label
 *  and the full explanation. */
const DELETE_LABELS: Record<string, { label: string; explanation: string }> = {
  trash_local: {
    label: "Moved to Trash",
    explanation: "Deleted in OneDrive, so moved to the Trash on this device.",
  },
  delete_remote: {
    label: "Deleted from OneDrive",
    explanation: "Deleted on this device, so deleted in OneDrive (its recycle bin keeps it).",
  },
};

function runningRows(runtimeStatus: SyncRuntimeAccountStatus): ActivityRow[] {
  if (runtimeStatus.phase === "paused") {
    return [];
  }
  return runtimeStatus.inProgress
    .filter((transfer) => transfer.state !== "queued")
    .sort((left, right) => Date.parse(left.startedAt) - Date.parse(right.startedAt) || left.path.localeCompare(right.path))
    .map((transfer) => {
      const verb = transfer.direction.toLowerCase() === "upload" ? "Uploading" : "Downloading";
      return {
        id: transfer.id,
        kind: "running",
        direction: transfer.direction,
        path: transfer.path,
        detail:
          transfer.bytesTotal === null
            ? "Size unknown"
            : transfer.bytesDone > 0
              ? `${formatBytes(transfer.bytesDone)} of ${formatBytes(transfer.bytesTotal)}`
              : formatBytes(transfer.bytesTotal),
        when: "",
        explanation: `${verb} ${transfer.path}`,
        progressPercent: percentOf(transfer.bytesDone, transfer.bytesTotal),
      };
    });
}

function finishedRow(item: SyncRuntimeRecentItem, failed: boolean): ActivityRow {
  const when = formatClockTime(item.finishedAt);
  const isUpload = item.direction.toLowerCase() === "upload";
  if (failed) {
    const verb = isUpload ? "upload" : "download";
    return {
      id: `failed-${item.id}`,
      kind: "failed",
      direction: item.direction,
      path: item.path,
      detail: `Couldn't ${verb}`,
      when,
      explanation: `Couldn't ${verb} ${item.path}. Details and Retry are in Sync problems.`,
      progressPercent: null,
    };
  }
  const deleted = DELETE_LABELS[item.direction];
  if (deleted) {
    return {
      id: item.id,
      kind: "deleted",
      direction: item.direction,
      path: item.path,
      detail: deleted.label,
      when,
      explanation: `${item.path}: ${deleted.explanation}`,
      progressPercent: null,
    };
  }
  const verb = isUpload ? "Uploaded" : "Downloaded";
  return {
    id: item.id,
    kind: "done",
    direction: item.direction,
    path: item.path,
    detail: `${verb} · ${formatBytes(item.bytesTotal)}`,
    when,
    explanation: `${verb} ${item.path}`,
    progressPercent: null,
  };
}

/** Everything the sync is doing and did in the last 10 minutes: running
 *  transfers first, then finished ones (transfers, deletes, failures),
 *  newest first. */
function feedRows(runtimeStatus: SyncRuntimeAccountStatus): ActivityRow[] {
  const finished = [
    ...runtimeStatus.recentCompleted.map((item) => ({ item, failed: false })),
    ...runtimeStatus.recentFailed.map((item) => ({ item, failed: true })),
  ]
    .sort((left, right) => Date.parse(right.item.finishedAt) - Date.parse(left.item.finishedAt))
    .slice(0, FINISHED_LIMIT)
    .map(({ item, failed }) => finishedRow(item, failed));
  return [...runningRows(runtimeStatus), ...finished];
}

function RowStateIcon({ row }: { row: ActivityRow }) {
  switch (row.kind) {
    case "done":
      return <IconCircleCheckFilled size={17} />;
    case "deleted":
      return <IconTrash size={17} />;
    case "failed":
      return <IconAlertTriangle size={17} />;
    default:
      return row.direction.toLowerCase() === "upload" ? <IconUpload size={17} /> : <IconDownload size={17} />;
  }
}

function ActivityRowView({ row, onOpen }: { row: ActivityRow; onOpen: (path: string) => Promise<void> }) {
  const { name, folder } = splitSyncPath(row.path);
  return (
    <button
      type="button"
      class="sync-file-open sync-activity-row"
      title={row.explanation}
      onClick={() => void onOpen(row.path)}
    >
      <span class="sync-file-icon">
        <FileTypeIcon path={row.path} size={20} />
      </span>
      <span class="sync-file-text">
        <span class="sync-file-name">{name}</span>
        <span class="sync-file-folder">{folder || "OneDrive"}</span>
      </span>
      <span class="sync-activity-size">
        <span class="sync-activity-detail">{row.detail}</span>
        {row.progressPercent !== null ? (
          <span class="sync-activity-mini-track">
            <span class="sync-activity-mini-fill" style={{ width: `${row.progressPercent.toFixed(0)}%` }} />
          </span>
        ) : (
          row.when && <span class="sync-activity-when">{row.when}</span>
        )}
      </span>
      <span class={`sync-activity-state is-${row.kind}`}>
        <RowStateIcon row={row} />
      </span>
    </button>
  );
}

/** Counts above this show as "99+" (exact number in the tooltip). */
const TAB_COUNT_SHOWN_MAX = 99;

/**
 * A tab's count: always shown (0 included) at one fixed width, so a count
 * changing never moves the tabs beside it.
 */
function TabCount({ count, attention = false }: { count: number; attention?: boolean }) {
  const shown = count > TAB_COUNT_SHOWN_MAX ? `${TAB_COUNT_SHOWN_MAX}+` : String(count);
  return (
    <span
      class={attention ? "sync-activity-count is-attention" : "sync-activity-count"}
      title={count > TAB_COUNT_SHOWN_MAX ? count.toLocaleString() : undefined}
    >
      {shown}
    </span>
  );
}

function TabButton({
  tab,
  current,
  onSelect,
  title,
  children,
}: {
  tab: ActivityTab;
  current: ActivityTab;
  onSelect: (tab: ActivityTab) => void;
  title?: string;
  children: ComponentChildren;
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={current === tab}
      class={current === tab ? "sync-activity-tab is-active" : "sync-activity-tab"}
      onClick={() => onSelect(tab)}
      title={title}
    >
      {children}
    </button>
  );
}

export function AccountSyncActivityList({
  runtimeStatus,
  summary,
  transfersMoving,
  problemCount,
  problems,
  notSyncedCount,
  notSynced,
  cleanup,
  onOpenItemFolder,
}: AccountSyncActivityListProps) {
  // Chosen once, when the page opens; it never switches under the user.
  const [tab, setTab] = useState<ActivityTab>(() => (transfersMoving ? "activity" : "summary"));
  const listBodyRef = useRef<HTMLDivElement>(null);
  const listBodyHeight = useElementHeight(listBodyRef);
  const rows = runtimeStatus ? feedRows(runtimeStatus) : [];
  const emptyText =
    runtimeStatus?.phase === "paused"
      ? "Paused. Nothing is transferring."
      : "Up to date · nothing changed in the last 10 minutes.";

  return (
    <section class="sync-activity">
      <div class="sync-activity-tabs" role="tablist" aria-label="Sync activity and details">
        <TabButton tab="summary" current={tab} onSelect={setTab} title="Where this account's sync is up to">
          Summary
        </TabButton>
        <TabButton tab="activity" current={tab} onSelect={setTab} title="What SomeDrive is doing and did in the last 10 minutes">
          Activity
        </TabButton>
        <TabButton tab="problems" current={tab} onSelect={setTab}>
          Sync problems{" "}
          <TabCount count={problemCount} attention={problemCount > 0} />
        </TabButton>
        <TabButton
          tab="notSynced"
          current={tab}
          onSelect={setTab}
          title="Items skipped on purpose, such as OneNote notebooks and shared folders"
        >
          Not synced <TabCount count={notSyncedCount} />
        </TabButton>
        <TabButton
          tab="cleanup"
          current={tab}
          onSelect={setTab}
          title="Identical copies and large files you could delete to free up OneDrive space"
        >
          Clean up
        </TabButton>
        <TabButton tab="details" current={tab} onSelect={setTab}>
          Details
        </TabButton>
      </div>
      <div class="sync-activity-body" ref={listBodyRef}>
        {tab === "summary" ? (
          <div class="sync-tab-scroll">{summary}</div>
        ) : tab === "problems" ? (
          <div class="sync-tab-scroll">{problems}</div>
        ) : tab === "notSynced" ? (
          <div class="sync-tab-scroll">{notSynced}</div>
        ) : tab === "cleanup" ? (
          <div class="sync-tab-scroll">{cleanup}</div>
        ) : tab === "details" ? (
          runtimeStatus ? (
            <AccountSyncDetails runtimeStatus={runtimeStatus} />
          ) : (
            <p class="sync-activity-empty">No status has been received for this account yet.</p>
          )
        ) : rows.length === 0 ? (
          <p class="sync-activity-empty">{emptyText}</p>
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
