import { IconEye, IconRefresh } from "@tabler/icons-preact";
import { useCallback, useEffect, useRef, useState } from "preact/hooks";
import type { CleanupActions, CleanupView, DuplicateSuggestion, LargeFile } from "../../types/somedrive";
import { CleanupDeleteDialog } from "./CleanupDeleteDialog";
import { DuplicateDeleteSummary, DuplicateSuggestionItem, LargeFileDeleteSummary, LargeFileItem } from "./CleanupItems";
import { formatBytes } from "./syncFormatting";

/** Suggestions smaller than this are tucked behind a "smaller" toggle. */
const SMALL_SUGGESTION_BYTES = 1024 * 1024;

type PendingDelete = { kind: "duplicates"; suggestion: DuplicateSuggestion } | { kind: "large"; file: LargeFile };

interface AccountCleanupProps {
  accountId: string;
  actions: CleanupActions;
  onOpenItemFolder: (relativePath: string) => Promise<void>;
}

function plural(count: number, noun: string): string {
  return `${count.toLocaleString()} ${noun}${count === 1 ? "" : "s"}`;
}

/**
 * The Clean up tab: identical copies (as whole folders where possible) and
 * the largest files, each deletable after a typed confirmation. Read when
 * the tab opens and after every change.
 */
export function AccountCleanup({ accountId, actions, onOpenItemFolder }: AccountCleanupProps) {
  const [view, setView] = useState<CleanupView | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [showSmall, setShowSmall] = useState(false);
  const [pending, setPending] = useState<PendingDelete | null>(null);
  const actionsRef = useRef(actions);
  actionsRef.current = actions;

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      setView(await actionsRef.current.list(accountId));
      setLoadError(null);
    } catch (reason) {
      setLoadError(String(reason));
    } finally {
      setLoading(false);
    }
  }, [accountId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const hide = async (path: string) => {
    await actionsRef.current.hide(accountId, path);
    await refresh();
  };
  const unhide = async (path: string) => {
    await actionsRef.current.unhide(accountId, path);
    await refresh();
  };

  if (!view) {
    return loadError ? (
      <p class="sync-activity-empty">Couldn't look for duplicates: {loadError}</p>
    ) : (
      <p class="sync-activity-empty">Looking for identical copies and large files…</p>
    );
  }

  const bigSuggestions = view.duplicates.filter((suggestion) => suggestion.bytes >= SMALL_SUGGESTION_BYTES);
  const smallSuggestions = view.duplicates.filter((suggestion) => suggestion.bytes < SMALL_SUGGESTION_BYTES);
  const smallBytes = smallSuggestions.reduce((total, suggestion) => total + suggestion.bytes, 0);
  const shownSuggestions = showSmall ? view.duplicates : bigSuggestions;

  return (
    <div class="sync-problems cleanup">
      <div class="sync-problems-bulk">
        <span class="sync-problems-note">
          {view.duplicates.length === 0
            ? "No identical copies found."
            : `${formatBytes(view.duplicateBytes)} of identical copies could be freed.`}
        </span>
        <button type="button" class="ghost" disabled={loading} onClick={() => void refresh()}>
          <IconRefresh size={15} /> Refresh
        </button>
      </div>
      <p class="sync-problems-note">
        A copy has the same name and exactly the same content as a file that stays. Each one is compared byte for
        byte before anything is deleted. Deleted files go to the Trash here, and OneDrive keeps them in its recycle
        bin.
      </p>

      {shownSuggestions.length > 0 && (
        <section class="cleanup-section" aria-label="Identical copies">
          <h4 class="cleanup-heading">Identical copies</h4>
          <ul class="sync-problem-list">
            {shownSuggestions.map((suggestion) => (
              <DuplicateSuggestionItem
                key={suggestion.id}
                suggestion={suggestion}
                onDelete={() => setPending({ kind: "duplicates", suggestion })}
                onHide={() => void hide(suggestion.path)}
                onOpenItemFolder={onOpenItemFolder}
              />
            ))}
          </ul>
        </section>
      )}
      {smallSuggestions.length > 0 && (
        <button type="button" class="ghost cleanup-toggle" onClick={() => setShowSmall(!showSmall)}>
          {showSmall
            ? "Hide the smaller suggestions"
            : `Show ${plural(smallSuggestions.length, "smaller suggestion")} (${formatBytes(smallBytes)} in all)`}
        </button>
      )}

      {view.largeFiles.length > 0 && (
        <section class="cleanup-section" aria-label="Large files">
          <h4 class="cleanup-heading">Large files</h4>
          <ul class="sync-problem-list">
            {view.largeFiles.map((file) => (
              <LargeFileItem
                key={file.path}
                file={file}
                onDelete={() => setPending({ kind: "large", file })}
                onHide={() => void hide(file.path)}
                onOpenItemFolder={onOpenItemFolder}
              />
            ))}
          </ul>
        </section>
      )}

      {view.hidden.length > 0 && (
        <details class="cleanup-section cleanup-hidden">
          <summary class="cleanup-heading">Hidden from Clean up ({view.hidden.length})</summary>
          <ul class="cleanup-hidden-list">
            {view.hidden.map((path) => (
              <li key={path}>
                <span class="cleanup-hidden-path">{path || "OneDrive"}</span>
                <button type="button" class="ghost" onClick={() => void unhide(path)}>
                  <IconEye size={15} /> Show again
                </button>
              </li>
            ))}
          </ul>
        </details>
      )}

      {pending?.kind === "duplicates" && (
        <CleanupDeleteDialog
          title="Delete identical copies?"
          summary={<DuplicateDeleteSummary suggestion={pending.suggestion} />}
          confirmLabel={`Delete ${plural(pending.suggestion.fileCount, "file")}`}
          busyText="Comparing every file with its copy, then moving them to the Trash…"
          onConfirm={async () => {
            await actionsRef.current.deleteDuplicates(accountId, pending.suggestion.id);
            await refresh();
          }}
          onClose={() => setPending(null)}
        />
      )}
      {pending?.kind === "large" && (
        <CleanupDeleteDialog
          title="Delete this file?"
          summary={<LargeFileDeleteSummary file={pending.file} />}
          confirmLabel="Delete file"
          busyText="Moving the file to the Trash…"
          onConfirm={async () => {
            await actionsRef.current.deleteLargeFile(accountId, pending.file.path);
            await refresh();
          }}
          onClose={() => setPending(null)}
        />
      )}
    </div>
  );
}
