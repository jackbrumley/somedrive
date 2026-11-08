import type { ComponentChildren } from "preact";
import { IconX } from "@tabler/icons-preact";
import { useId, useLayoutEffect, useRef } from "preact/hooks";

interface ModalProps {
  title: string;
  onClose: () => void;
  dismissible: boolean;
  children: ComponentChildren;
}

// An accessible dialog (App Standard section 6): labelled by its heading,
// closes on Escape, takes focus when it opens and returns it when it closes.
export function Modal({ title, onClose, dismissible, children }: ModalProps) {
  const titleId = useId();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const backdropPointerDown = useRef(false);
  // Captured on the first render, before any child can autoFocus itself.
  const returnFocusRef = useRef(document.activeElement instanceof HTMLElement ? document.activeElement : null);

  useLayoutEffect(() => {
    const dialog = dialogRef.current!;
    const previouslyFocused = returnFocusRef.current;
    // The top layer escapes ancestor clipping and stacking contexts. The
    // browser manages focus and makes the background inert.
    dialog.showModal();

    return () => {
      dialog.close();
      if (previouslyFocused?.isConnected) {
        previouslyFocused.focus();
      }
    };
  }, []);

  return (
    <dialog
      ref={dialogRef}
      class="modal-overlay"
      aria-modal="true"
      aria-labelledby={titleId}
      onCancel={(event) => {
        // The caller owns visibility, including refusing dismissal while busy.
        event.preventDefault();
        if (dismissible) onClose();
      }}
      onPointerDown={(event) => {
        backdropPointerDown.current = event.target === event.currentTarget;
      }}
      onClick={(event) => {
        if (dismissible && backdropPointerDown.current && event.target === event.currentTarget) {
          onClose();
        }
        backdropPointerDown.current = false;
      }}
    >
      <section class="modal-shell" tabIndex={-1}>
        <header class="modal-header">
          <h3 id={titleId}>{title}</h3>
          <button type="button" class="modal-close" onClick={onClose} disabled={!dismissible} aria-label="Close dialog">
            <IconX size={16} stroke={2.2} />
          </button>
        </header>
        <div class="modal-body">{children}</div>
      </section>
    </dialog>
  );
}
