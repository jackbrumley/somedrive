import {
  IconAlertTriangle,
  IconChevronDown,
  IconChevronRight,
  IconEyeOff,
  IconFiles,
  IconFolder,
  IconShieldCheck,
  IconTrash,
} from "@tabler/icons-preact";
import { useState } from "preact/hooks";
import type { DuplicateSuggestion, KeptLocation, LargeFile } from "../../types/somedrive";
import { FileTypeIcon } from "./FileTypeIcon";
import { formatBytes, splitSyncPath } from "./syncFormatting";

function plural(count: number, noun: string): string {
  return `${count.toLocaleString()} ${noun}${count === 1 ? "" : "s"}`;
}

function placeName(path: string): string {
  return path || "OneDrive";
}

/** "Games/Backup/20250809 (8,942 files), … and 2 more places". */
function keptText(keptIn: KeptLocation[], more: number): string {
  const places = keptIn.map((kept) => `${placeName(kept.path)} (${plural(kept.fileCount, "file")})`).join(", ");
  return more > 0 ? `${places} and ${plural(more, "more place")}` : places;
}

/** What the suggestion covers, in a sentence. */
function describe(suggestion: DuplicateSuggestion): string {
  const { fileCount, folderFileCount } = suggestion;
  switch (suggestion.kind) {
    case "folder":
      return fileCount === 1
        ? "Whole folder: its only file is an identical copy."
        : `Whole folder: all ${plural(fileCount, "file")} are identical copies.`;
    case "partial":
      return `${fileCount.toLocaleString()} of its ${plural(folderFileCount, "file")} are identical copies. The other ${plural(folderFileCount - fileCount, "file")} differ and stay.`;
    default:
      return `${fileCount.toLocaleString()} of the ${plural(folderFileCount, "file")} directly in this folder are identical copies.`;
  }
}

/** The name line and the folder line of a suggestion. */
function headline(suggestion: DuplicateSuggestion): { name: string; folder: string } {
  if (suggestion.kind === "files") {
    return { name: plural(suggestion.fileCount, "identical file"), folder: placeName(suggestion.path) };
  }
  const { name, folder } = splitSyncPath(suggestion.path);
  return { name, folder: placeName(folder) };
}

function relativeTo(path: string, folder: string): string {
  return folder && path.startsWith(`${folder}/`) ? path.slice(folder.length + 1) : path;
}

interface DuplicateSuggestionItemProps {
  suggestion: DuplicateSuggestion;
  onDelete: () => void;
  onHide: () => void;
  onOpenItemFolder: (relativePath: string) => Promise<void>;
}

/** One duplicate suggestion: what goes, where the copies stay, and its files. */
export function DuplicateSuggestionItem({ suggestion, onDelete, onHide, onOpenItemFolder }: DuplicateSuggestionItemProps) {
  const [expanded, setExpanded] = useState(false);
  const { name, folder } = headline(suggestion);
  const notListed = suggestion.fileCount - suggestion.pairs.length;
  return (
    <li class="sync-problem cleanup-item">
      <div class="sync-problem-head">
        <span class="sync-problem-icon cleanup-icon">
          {suggestion.kind === "files" ? <IconFiles size={18} /> : <IconFolder size={18} />}
        </span>
        <button type="button" class="sync-problem-name" onClick={() => void onOpenItemFolder(suggestion.path)}>
          <span class="sync-file-name">{name}</span>
          <span class="sync-file-folder">{folder}</span>
        </button>
        <span class="cleanup-size">{formatBytes(suggestion.bytes)}</span>
      </div>
      <p class="sync-problem-detail">{describe(suggestion)}</p>
      <p class="cleanup-kept">
        <IconShieldCheck size={15} />
        <span>Identical copies stay in {keptText(suggestion.keptIn, suggestion.keptInMore)}.</span>
      </p>
      {expanded && (
        <ul class="cleanup-pairs">
          {suggestion.pairs.map((pair) => (
            <li key={pair.path}>
              <span class="cleanup-pair-path" title={pair.path}>
                {relativeTo(pair.path, suggestion.path)}
              </span>
              <span class="cleanup-pair-kept" title={pair.keptPath}>
                same as {pair.keptPath}
              </span>
              <span class="cleanup-pair-size">{formatBytes(pair.size)}</span>
            </li>
          ))}
          {notListed > 0 && <li class="cleanup-pairs-more">and {plural(notListed, "smaller file")}</li>}
        </ul>
      )}
      <div class="sync-problem-actions">
        <button type="button" class="danger" onClick={onDelete}>
          <IconTrash size={15} /> Delete…
        </button>
        <button type="button" class="ghost" aria-expanded={expanded} onClick={() => setExpanded(!expanded)}>
          {expanded ? <IconChevronDown size={15} /> : <IconChevronRight size={15} />}
          {expanded ? "Hide files" : "Show files"}
        </button>
        {suggestion.path !== "" && (
          <button type="button" class="ghost" title="Stop suggesting this folder" onClick={onHide}>
            <IconEyeOff size={15} /> Hide
          </button>
        )}
      </div>
    </li>
  );
}

interface LargeFileItemProps {
  file: LargeFile;
  onDelete: () => void;
  onHide: () => void;
  onOpenItemFolder: (relativePath: string) => Promise<void>;
}

/** One large file, with whether an identical copy exists. */
export function LargeFileItem({ file, onDelete, onHide, onOpenItemFolder }: LargeFileItemProps) {
  const { name, folder } = splitSyncPath(file.path);
  return (
    <li class="sync-problem cleanup-item">
      <div class="sync-problem-head">
        <span class="sync-problem-icon cleanup-icon">
          <FileTypeIcon path={file.path} size={18} />
        </span>
        <button type="button" class="sync-problem-name" onClick={() => void onOpenItemFolder(file.path)}>
          <span class="sync-file-name">{name}</span>
          <span class="sync-file-folder">{placeName(folder)}</span>
        </button>
        <span class="cleanup-size">{formatBytes(file.size)}</span>
      </div>
      <p class="sync-problem-detail">
        {file.copies.length > 0
          ? `An identical copy is also at ${file.copies[0]}${file.copies.length > 1 ? ` (and ${plural(file.copies.length - 1, "other")})` : ""}.`
          : "The only copy of this file in your OneDrive."}
      </p>
      <div class="sync-problem-actions">
        <button type="button" class="danger" onClick={onDelete}>
          <IconTrash size={15} /> Delete…
        </button>
        <button type="button" class="ghost" title="Stop listing this file" onClick={onHide}>
          <IconEyeOff size={15} /> Hide
        </button>
      </div>
    </li>
  );
}

/** The confirmation text for a duplicate suggestion. */
export function DuplicateDeleteSummary({ suggestion }: { suggestion: DuplicateSuggestion }) {
  const what =
    suggestion.kind === "folder"
      ? `the folder ${suggestion.path}`
      : suggestion.kind === "partial"
        ? `the identical copies in ${suggestion.path}`
        : `${plural(suggestion.fileCount, "identical file")} in ${placeName(suggestion.path)}`;
  const differing = suggestion.folderFileCount - suggestion.fileCount;
  return (
    <>
      <p>
        This deletes <strong>{what}</strong>: {plural(suggestion.fileCount, "file")}, {formatBytes(suggestion.bytes)}.
        {suggestion.kind === "partial" && ` The ${plural(differing, "file")} that differ stay.`}
      </p>
      <p class="cleanup-confirm-safe">
        <IconShieldCheck size={16} />
        <span>
          Every one of these files has an identical copy that stays, in{" "}
          {keptText(suggestion.keptIn, suggestion.keptInMore)}. SomeDrive compares each file with its copy byte for
          byte first; if anything differs, nothing is deleted.
        </span>
      </p>
      <p class="sync-problem-detail">
        They go to the Trash on this device, and OneDrive keeps them in its recycle bin.
      </p>
    </>
  );
}

/** The confirmation text for a large file. */
export function LargeFileDeleteSummary({ file }: { file: LargeFile }) {
  return (
    <>
      <p>
        This deletes <strong>{file.path}</strong> ({formatBytes(file.size)}).
      </p>
      {file.copies.length > 0 ? (
        <p class="cleanup-confirm-safe">
          <IconShieldCheck size={16} />
          <span>An identical copy stays at {file.copies[0]}.</span>
        </p>
      ) : (
        <p class="cleanup-confirm-warning">
          <IconAlertTriangle size={16} />
          <span>This is the only copy of this file in your OneDrive.</span>
        </p>
      )}
      <p class="sync-problem-detail">It goes to the Trash on this device, and OneDrive keeps it in its recycle bin.</p>
    </>
  );
}
