import type { ComponentChildren } from "preact";
import { IconX } from "@tabler/icons-preact";
import { useEffect, useId, useRef } from "preact/hooks";

interface ModalProps {
  title: string;
  onClose: () => void;
  children: ComponentChildren;
}

// An accessible dialog (App Standard section 6): labelled by its heading,
// closes on Escape, takes focus when it opens and returns it when it closes.
export function Modal({ title, onClose, children }: ModalProps) {
  const titleId = useId();
  const shellRef = useRef<HTMLElement | null>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  // Captured on the first render, before any child can autoFocus itself.
  const returnFocusRef = useRef(document.activeElement instanceof HTMLElement ? document.activeElement : null);

  useEffect(() => {
    const { body } = document;
    const currentCount = Number.parseInt(body.dataset.modalOpenCount ?? "0", 10);
    const nextCount = Number.isNaN(currentCount) ? 1 : currentCount + 1;

    body.dataset.modalOpenCount = String(nextCount);
    body.classList.add("app-modal-open");

    return () => {
      const previousCount = Number.parseInt(body.dataset.modalOpenCount ?? "1", 10);
      const updatedCount = Number.isNaN(previousCount) ? 0 : Math.max(0, previousCount - 1);

      if (updatedCount === 0) {
        delete body.dataset.modalOpenCount;
        body.classList.remove("app-modal-open");
        return;
      }

      body.dataset.modalOpenCount = String(updatedCount);
    };
  }, []);

  useEffect(() => {
    const previouslyFocused = returnFocusRef.current;
    // A child may already have focused itself (autoFocus); otherwise focus the dialog.
    if (!shellRef.current?.contains(document.activeElement)) {
      shellRef.current?.focus();
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        onCloseRef.current();
      }
    };
    document.addEventListener("keydown", handleKeyDown);

    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      if (previouslyFocused?.isConnected) {
        previouslyFocused.focus();
      }
    };
  }, []);

  return (
    <div class="modal-overlay" onClick={onClose}>
      <section
        ref={shellRef}
        class="modal-shell"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        onClick={(event) => event.stopPropagation()}
      >
        <header class="modal-header">
          <h3 id={titleId}>{title}</h3>
          <button class="modal-close" onClick={onClose} aria-label="Close dialog">
            <IconX size={16} stroke={2.2} />
          </button>
        </header>
        <div class="modal-body">{children}</div>
      </section>
    </div>
  );
}
