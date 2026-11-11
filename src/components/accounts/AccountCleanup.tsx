import { IconAlertTriangle, IconEye } from "@tabler/icons-preact";
import { useRef, useState } from "preact/hooks";
import { useAccountAreas } from "../../state/accountAreas";
import type { CleanupActions, CleanupView, DuplicateSuggestion, LargeFile } from "../../types/somedrive";
import { CleanupDeleteDialog } from "./CleanupDeleteDialog";
import { DuplicateDeleteSummary, DuplicateSuggestionItem, LargeFileDeleteSummary, LargeFileItem } from "./CleanupItems";
import { formatBytes, splitSyncPath } from "./syncFormatting";

/** Suggestions smaller than this are tucked behind a "smaller" toggle. */
const SMALL_SUGGESTION_BYTES = 1024 * 1024;

/** The word typed to confirm deleting identical copies. */
const COPIES_CONFIRM_WORD = "delete";

/** The two lists, shown one at a time so a large file is never deleted from a list of copies. */
type CleanupList = "copies" | "large";

type PendingDelete = { kind: "duplicates"; suggestion: DuplicateSuggestion } | { kind: "large"; file: LargeFile };

interface AccountCleanupProps {
  accountId: string;
  /** The account's Clean up change counter: the list is reread when it moves. */
  revision: number | undefined;
  actions: CleanupActions;
  onOpenItemFolder: (relativePath: string) => Promise<void>;
}

function plural(count: number, noun: string): string {
  return `${count.toLocaleString()} ${noun}${count === 1 ? "" : "s"}`;
}

/** "Delete 1 copy" / "Delete 201 copies". */
function copiesLabel(count: number): string {
  return `Delete ${count.toLocaleString()} ${count === 1 ? "copy" : "copies"}`;
}

/**
 * The Clean up tab: identical copies (as whole folders where possible) and
 * the largest files, as two separate lists. Each delete needs a typed
 * confirmation. Read from the store while the tab is open; it updates by
 * itself after a delete, a hide or a sync change.
 */
export function AccountCleanup({ accountId, revision, actions, onOpenItemFolder }: AccountCleanupProps) {
  const { data: view, error: loadError } = useAccountAreas().useCleanup(accountId, revision);
  const [list, setList] = useState<CleanupList>("copies");
  const [showSmall, setShowSmall] = useState(false);
  const [pending, setPending] = useState<PendingDelete | null>(null);
  const actionsRef = useRef(actions);
  actionsRef.current = actions;

  const hide = (path: string) => actionsRef.current.hide(accountId, path);
  const unhide = (path: string) => actionsRef.current.unhide(accountId, path);

  if (!view) {
    return loadError ? (
      <p class="sync-activity-empty">Couldn't look for duplicates: {loadError}</p>
    ) : (
      <p class="sync-activity-empty">Looking for identical copies and large files…</p>
    );
  }

  return (
    <div class="sync-problems cleanup">
      <div class="cleanup-switch" role="group" aria-label="Which list to show">
        <button
          type="button"
          class={list === "copies" ? "cleanup-switch-option is-active" : "cleanup-switch-option"}
          aria-pressed={list === "copies"}
          onClick={() => setList("copies")}
        >
          Identical copies <span class="cleanup-switch-count">{formatBytes(view.duplicateBytes)}</span>
        </button>
        <button
          type="button"
          class={list === "large" ? "cleanup-switch-option is-active" : "cleanup-switch-option"}
          aria-pressed={list === "large"}
          onClick={() => setList("large")}
        >
          Large files <span class="cleanup-switch-count">{view.largeFiles.length}</span>
        </button>
      </div>
      {loadError && <p class="sync-problems-note">Couldn't update this list: {loadError}</p>}

      {list === "copies" ? (
        <CopiesList
          view={view}
          showSmall={showSmall}
          onToggleSmall={() => setShowSmall(!showSmall)}
          onDelete={(suggestion) => setPending({ kind: "duplicates", suggestion })}
          onHide={(path) => void hide(path)}
          onOpenItemFolder={onOpenItemFolder}
        />
      ) : (
        <LargeFilesList
          files={view.largeFiles}
          onOpenFile={(path) => void actionsRef.current.openFile(accountId, path)}
          onDelete={(file) => setPending({ kind: "large", file })}
          onHide={(path) => void hide(path)}
          onShowCopies={() => setList("copies")}
          onOpenItemFolder={onOpenItemFolder}
        />
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
          confirmWord={COPIES_CONFIRM_WORD}
          confirmLabel={copiesLabel(pending.suggestion.fileCount)}
          busyText="Comparing every file with its copy, then moving them to the Trash…"
          onConfirm={async () => {
            await actionsRef.current.deleteDuplicates(accountId, pending.suggestion.id);
          }}
          onClose={() => setPending(null)}
        />
      )}
      {pending?.kind === "large" && (
        <CleanupDeleteDialog
          title="Delete the only copy of this file?"
          summary={<LargeFileDeleteSummary file={pending.file} />}
          confirmWord={splitSyncPath(pending.file.path).name}
          onCopyConfirmWord={(word) => actionsRef.current.copyText(word)}
          confirmLabel="Delete file"
          busyText="Moving the file to the Trash…"
          onConfirm={async () => {
            await actionsRef.current.deleteLargeFile(accountId, pending.file.path);
          }}
          onClose={() => setPending(null)}
        />
      )}
    </div>
  );
}

interface CopiesListProps {
  view: CleanupView;
  showSmall: boolean;
  onToggleSmall: () => void;
  onDelete: (suggestion: DuplicateSuggestion) => void;
  onHide: (path: string) => void;
  onOpenItemFolder: (relativePath: string) => Promise<void>;
}

/** Identical copies: every file listed has a copy that stays. */
function CopiesList({ view, showSmall, onToggleSmall, onDelete, onHide, onOpenItemFolder }: CopiesListProps) {
  const bigSuggestions = view.duplicates.filter((suggestion) => suggestion.bytes >= SMALL_SUGGESTION_BYTES);
  const smallSuggestions = view.duplicates.filter((suggestion) => suggestion.bytes < SMALL_SUGGESTION_BYTES);
  const smallBytes = smallSuggestions.reduce((total, suggestion) => total + suggestion.bytes, 0);
  const shownSuggestions = showSmall ? view.duplicates : bigSuggestions;
  return (
    <>
      <p class="sync-problems-note">
        {view.duplicates.length === 0
          ? "No identical copies found."
          : `${formatBytes(view.duplicateBytes)} of identical copies could be freed. `}
        A copy has the same name and exactly the same content as a file that stays. Each one is compared byte for byte
        before anything is deleted. Deleted files go to the Trash here, and OneDrive keeps them in its recycle bin.
      </p>
      {shownSuggestions.length > 0 && (
        <ul class="sync-problem-list" aria-label="Identical copies">
          {shownSuggestions.map((suggestion) => (
            <DuplicateSuggestionItem
              key={suggestion.id}
              suggestion={suggestion}
              onDelete={() => onDelete(suggestion)}
              onHide={() => onHide(suggestion.path)}
              onOpenItemFolder={onOpenItemFolder}
            />
          ))}
        </ul>
      )}
      {smallSuggestions.length > 0 && (
        <button type="button" class="ghost cleanup-toggle" onClick={onToggleSmall}>
          {showSmall
            ? "Hide the smaller suggestions"
            : `Show ${plural(smallSuggestions.length, "smaller suggestion")} (${formatBytes(smallBytes)} in all)`}
        </button>
      )}
    </>
  );
}

interface LargeFilesListProps {
  files: LargeFile[];
  onOpenFile: (path: string) => void;
  onDelete: (file: LargeFile) => void;
  onHide: (path: string) => void;
  onShowCopies: () => void;
  onOpenItemFolder: (relativePath: string) => Promise<void>;
}

/** The largest files. Most are the only copy, so this list is not about duplicates. */
function LargeFilesList({ files, onOpenFile, onDelete, onHide, onShowCopies, onOpenItemFolder }: LargeFilesListProps) {
  return (
    <>
      <p class="cleanup-large-warning">
        <IconAlertTriangle size={16} />
        <span>
          These are not copies. Most are the only copy of the file in your OneDrive: deleting one removes it for good
          once OneDrive's recycle bin is emptied.
        </span>
      </p>
      {files.length === 0 ? (
        <p class="sync-activity-empty">No files over 100 MB.</p>
      ) : (
        <ul class="sync-problem-list" aria-label="Large files">
          {files.map((file) => (
            <LargeFileItem
              key={file.path}
              file={file}
              onOpen={() => onOpenFile(file.path)}
              onDelete={() => onDelete(file)}
              onHide={() => onHide(file.path)}
              onShowCopies={onShowCopies}
              onOpenItemFolder={onOpenItemFolder}
            />
          ))}
        </ul>
      )}
    </>
  );
}
