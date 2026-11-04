import { IconLoader2, IconTrash } from "@tabler/icons-preact";
import type { ComponentChildren } from "preact";
import { useState } from "preact/hooks";
import { Modal } from "../ui/Modal";

/** The word the user types to confirm a delete. */
const CONFIRM_WORD = "delete";

interface CleanupDeleteDialogProps {
  title: string;
  /** What is deleted and why it is safe (or that it is the only copy). */
  summary: ComponentChildren;
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
 * "delete". A refusal from the backend (a copy differs, a file is new) is
 * shown here and nothing is deleted.
 */
export function CleanupDeleteDialog({
  title,
  summary,
  confirmLabel,
  busyText,
  onConfirm,
  onClose,
}: CleanupDeleteDialogProps) {
  const [typed, setTyped] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ready = typed.trim().toLowerCase() === CONFIRM_WORD;

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
          <span>
            Type <strong>{CONFIRM_WORD}</strong> to confirm
          </span>
          <input
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
