import { IconAlertTriangle, IconArrowsDiff, IconRefresh } from "@tabler/icons-preact";
import { useState } from "preact/hooks";
import type { ConflictResolution, SyncProblem } from "../../types/somedrive";
import { formatBytes, splitSyncPath } from "./syncFormatting";

interface AccountSyncProblemsProps {
  problems: SyncProblem[];
  onResolve: (paths: string[], resolution: ConflictResolution) => Promise<void>;
  onRetryFailedDownload: (retryId: string, path: string) => Promise<void>;
  onRetryAllFailedDownloads: () => Promise<void>;
  onOpenItemFolder: (relativePath: string) => Promise<void>;
}

const RESOLUTION_LABELS: Record<ConflictResolution, string> = {
  keep_local: "Keeping this device's version",
  keep_cloud: "Keeping OneDrive's version",
  keep_both: "Keeping both",
};

function describeVersion(size: number | null, modifiedAt: string | null): string {
  if (size === null) {
    return "Not there";
  }
  const when = modifiedAt ? new Date(modifiedAt).toLocaleString() : "unknown time";
  return `${formatBytes(size)} · ${when}`;
}

function ResolutionButtons({
  disabled,
  onChoose,
}: {
  disabled: boolean;
  onChoose: (resolution: ConflictResolution) => void;
}) {
  return (
    <div class="sync-problem-actions">
      <button type="button" class="ghost" disabled={disabled} onClick={() => onChoose("keep_local")}>
        Keep this device's
      </button>
      <button type="button" class="ghost" disabled={disabled} onClick={() => onChoose("keep_cloud")}>
        Keep OneDrive's
      </button>
      <button type="button" class="ghost" disabled={disabled} onClick={() => onChoose("keep_both")}>
        Keep both
      </button>
    </div>
  );
}

/**
 * Files that need the user. Conflicts are held (not synced, not copied)
 * until a version is chosen, one file or many at once; the old version
 * stays recoverable in OneDrive's version history or the Trash.
 */
export function AccountSyncProblems({
  problems,
  onResolve,
  onRetryFailedDownload,
  onRetryAllFailedDownloads,
  onOpenItemFolder,
}: AccountSyncProblemsProps) {
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const openConflicts = problems.filter((problem) => problem.isConflict && problem.resolution === null);
  const failedDownloads = problems.filter((problem) => problem.retryId !== null);
  const selectedPaths = openConflicts.filter((problem) => selected.has(problem.path)).map((problem) => problem.path);
  const allSelected = openConflicts.length > 0 && selectedPaths.length === openConflicts.length;

  const resolve = (paths: string[], resolution: ConflictResolution) => {
    setBusy(true);
    void onResolve(paths, resolution).finally(() => {
      setSelected(new Set());
      setBusy(false);
    });
  };

  const toggle = (path: string) => {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  };

  if (problems.length === 0) {
    return <p class="sync-activity-empty">No problems. Everything is syncing normally.</p>;
  }

  return (
    <div class="sync-problems">
      {openConflicts.length > 0 && (
        <div class="sync-problems-bulk">
          <label class="sync-problems-select-all">
            <input
              type="checkbox"
              checked={allSelected}
              onChange={() =>
                setSelected(allSelected ? new Set() : new Set(openConflicts.map((problem) => problem.path)))
              }
            />
            {selectedPaths.length > 0 ? `${selectedPaths.length} selected` : "Select all"}
          </label>
          <ResolutionButtons
            disabled={busy || selectedPaths.length === 0}
            onChoose={(resolution) => resolve(selectedPaths, resolution)}
          />
        </div>
      )}
      {failedDownloads.length > 1 && (
        <div class="sync-problems-bulk">
          <span class="sync-problems-note">{failedDownloads.length} downloads failed.</span>
          <button type="button" class="ghost" onClick={() => void onRetryAllFailedDownloads()}>
            <IconRefresh size={15} />
            Retry all
          </button>
        </div>
      )}
      <ul class="sync-problem-list">
        {problems.map((problem) => {
          const { name, folder } = splitSyncPath(problem.path);
          const isOpenConflict = problem.isConflict && problem.resolution === null;
          return (
            <li key={`${problem.code}:${problem.path}`} class="sync-problem">
              <div class="sync-problem-head">
                {isOpenConflict ? (
                  <input
                    type="checkbox"
                    aria-label={`Select ${name}`}
                    checked={selected.has(problem.path)}
                    onChange={() => toggle(problem.path)}
                  />
                ) : (
                  <span class="sync-problem-icon">
                    {problem.isConflict ? <IconArrowsDiff size={17} /> : <IconAlertTriangle size={17} />}
                  </span>
                )}
                <button type="button" class="sync-problem-name" onClick={() => void onOpenItemFolder(problem.path)}>
                  <span class="sync-file-name">{name}</span>
                  <span class="sync-file-folder">{folder || "OneDrive"}</span>
                </button>
              </div>
              <p class="sync-problem-detail">{problem.detail}</p>
              {problem.isConflict && (
                <dl class="sync-problem-versions">
                  <div>
                    <dt>This device</dt>
                    <dd>{describeVersion(problem.localSize, problem.localModifiedAt)}</dd>
                  </div>
                  <div>
                    <dt>OneDrive</dt>
                    <dd>{describeVersion(problem.cloudSize, problem.cloudModifiedAt)}</dd>
                  </div>
                </dl>
              )}
              {isOpenConflict && (
                <ResolutionButtons disabled={busy} onChoose={(resolution) => resolve([problem.path], resolution)} />
              )}
              {problem.isConflict && problem.resolution !== null && (
                <p class="sync-problem-pending">{RESOLUTION_LABELS[problem.resolution]} on the next sync.</p>
              )}
              {problem.retryId !== null && (
                <div class="sync-problem-actions">
                  <button
                    type="button"
                    class="ghost"
                    onClick={() => void onRetryFailedDownload(problem.retryId as string, problem.path)}
                  >
                    <IconRefresh size={15} />
                    Retry
                  </button>
                </div>
              )}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
