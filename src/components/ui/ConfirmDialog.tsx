import { IconCheck, IconCopy, IconLoader2 } from "@tabler/icons-preact";
import type { ComponentChildren } from "preact";
import { useRef, useState } from "preact/hooks";
import { Modal } from "./Modal";

interface ConfirmDialogProps {
  title: string;
  /** What happens and what stays. */
  summary: ComponentChildren;
  /** Choices shown under the summary, e.g. a toggle. */
  options?: ComponentChildren;
  /**
   * When given, the confirm button only enables once the user types it
   * (ignoring case and surrounding spaces): for deletes that cannot be undone.
   */
  confirmWord?: string;
  /** When given, a Copy button puts the confirm word on the clipboard (a long name). */
  onCopyConfirmWord?: (word: string) => Promise<boolean>;
  /** The confirm button's text, e.g. "Delete 201 files". */
  confirmLabel: string;
  confirmIcon?: ComponentChildren;
  /** A red confirm button, for an action that removes something. */
  danger: boolean;
  /** Shown while the action runs. */
  busyText: string;
  /** Rejects with the reason when nothing was done; the dialog shows it. */
  onConfirm: () => Promise<void>;
  onClose: () => void;
}

/**
 * Asks before an action runs. The action runs inside the dialog: it stays open
 * (and cannot be dismissed) until the action finishes, and a refusal from the
 * backend is shown here.
 */
export function ConfirmDialog({
  title,
  summary,
  options,
  confirmWord,
  onCopyConfirmWord,
  confirmLabel,
  confirmIcon,
  danger,
  busyText,
  onConfirm,
  onClose,
}: ConfirmDialogProps) {
  const [typed, setTyped] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const ready = confirmWord === undefined || typed.trim().toLowerCase() === confirmWord.toLowerCase();

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
    if (confirmWord && onCopyConfirmWord && (await onCopyConfirmWord(confirmWord))) {
      setCopied(true);
      inputRef.current?.focus();
    }
  };

  return (
    <Modal title={title} onClose={onClose} dismissible={!busy}>
      <form
        class="confirm-dialog"
        onSubmit={(event) => {
          event.preventDefault();
          void confirm();
        }}
      >
        <div class="confirm-dialog-summary">{summary}</div>
        {options}
        {confirmWord !== undefined && (
          <label class="confirm-dialog-label">
            <span class="confirm-dialog-prompt">
              <span>
                Type <strong class="confirm-dialog-word">{confirmWord}</strong> to confirm
              </span>
              {onCopyConfirmWord && (
                <button
                  type="button"
                  class="confirm-dialog-copy"
                  disabled={busy}
                  title={copied ? "Copied: paste it into the box" : "Copy it, then paste it into the box"}
                  aria-label="Copy the confirm text"
                  onClick={() => void copyWord()}
                >
                  {copied ? <IconCheck size={16} /> : <IconCopy size={16} />}
                </button>
              )}
            </span>
            <input
              ref={inputRef}
              class="confirm-dialog-input"
              type="text"
              value={typed}
              disabled={busy}
              autoFocus
              autoComplete="off"
              spellcheck={false}
              onInput={(event) => setTyped(event.currentTarget.value)}
            />
          </label>
        )}
        {busy && (
          <p class="confirm-dialog-busy" role="status">
            <IconLoader2 size={16} class="spin" /> {busyText}
          </p>
        )}
        {error && (
          <p class="confirm-dialog-error" role="alert">
            {error}
          </p>
        )}
        <div class="button-row confirm-dialog-actions">
          <button type="button" class="ghost" disabled={busy} onClick={onClose}>
            Cancel
          </button>
          <button
            type="submit"
            class={danger ? "danger" : undefined}
            disabled={!ready || busy}
            autoFocus={confirmWord === undefined}
          >
            {confirmIcon} {confirmLabel}
          </button>
        </div>
      </form>
    </Modal>
  );
}
