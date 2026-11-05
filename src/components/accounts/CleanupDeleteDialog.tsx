import { IconCheck, IconCopy, IconLoader2, IconTrash } from "@tabler/icons-preact";
import type { ComponentChildren } from "preact";
import { useRef, useState } from "preact/hooks";
import { Modal } from "../ui/Modal";

interface CleanupDeleteDialogProps {
  title: string;
  /** What is deleted and why it is safe (or that it is the only copy). */
  summary: ComponentChildren;
  /** What the user types to confirm: "delete" for copies, the file's name for an only copy. */
  confirmWord: string;
  /** When given, a Copy button puts the confirm word on the clipboard (a long file name). */
  onCopyConfirmWord?: (word: string) => Promise<boolean>;
  /** The confirm button, e.g. "Delete 201 files". */
  confirmLabel: string;
  /** What runs while the delete happens. */
  busyText: string;
  /** Rejects with the reason when nothing was deleted. */
  onConfirm: () => Promise<void>;
  onClose: () => void;
}

/**
 * Confirms a Clean up delete: the button only enables once the user types
 * the confirm word (ignoring case and surrounding spaces). A refusal from the backend (a copy differs, a file is new) is
 * shown here and nothing is deleted.
 */
export function CleanupDeleteDialog({
  title,
  summary,
  confirmWord,
  onCopyConfirmWord,
  confirmLabel,
  busyText,
  onConfirm,
  onClose,
}: CleanupDeleteDialogProps) {
  const [typed, setTyped] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const ready = typed.trim().toLowerCase() === confirmWord.toLowerCase();

  const confirm = async () => {
    if (!ready || busy) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await onConfirm();
      onClose();
    } catch (reason) {
      setError(String(reason));
      setBusy(false);
    }
  };

  const copyWord = async () => {
    if (onCopyConfirmWord && (await onCopyConfirmWord(confirmWord))) {
      setCopied(true);
      inputRef.current?.focus();
    }
  };

  return (
    <Modal title={title} onClose={busy ? () => undefined : onClose}>
      <form
        class="cleanup-confirm"
        onSubmit={(event) => {
          event.preventDefault();
          void confirm();
        }}
      >
        <div class="cleanup-confirm-summary">{summary}</div>
        <label class="cleanup-confirm-label">
          <span class="cleanup-confirm-prompt">
            <span>
              Type <strong class="cleanup-confirm-word">{confirmWord}</strong> to confirm
            </span>
            {onCopyConfirmWord && (
              <button
                type="button"
                class="cleanup-confirm-copy"
                disabled={busy}
                title={copied ? "Copied: paste it into the box" : "Copy the name, then paste it into the box"}
                aria-label="Copy the name"
                onClick={() => void copyWord()}
              >
                {copied ? <IconCheck size={16} /> : <IconCopy size={16} />}
              </button>
            )}
          </span>
          <input
            ref={inputRef}
            class="cleanup-confirm-input"
            type="text"
            value={typed}
            disabled={busy}
            autoFocus
            autoComplete="off"
            spellcheck={false}
            onInput={(event) => setTyped(event.currentTarget.value)}
          />
        </label>
        {busy && (
          <p class="cleanup-confirm-busy" role="status">
            <IconLoader2 size={16} class="spin" /> {busyText}
          </p>
        )}
        {error && (
          <p class="cleanup-confirm-error" role="alert">
            {error}
          </p>
        )}
        <div class="button-row cleanup-confirm-actions">
          <button type="button" class="ghost" disabled={busy} onClick={onClose}>
            Cancel
          </button>
          <button type="submit" class="danger" disabled={!ready || busy}>
            <IconTrash size={15} /> {confirmLabel}
          </button>
        </div>
      </form>
    </Modal>
  );
}
