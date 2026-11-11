import { IconChevronLeft } from "@tabler/icons-preact";
import type { ComponentChildren } from "preact";

interface PageHeaderBack {
  href: string;
  label: string;
  onBack: () => void;
}

/** What the page's subject is doing, shown in the header on every tab: one
 *  line of text beside the actions, with a thin bar along the header's
 *  bottom edge while there is progress to show, or a small coloured dot
 *  for its state when there is not. */
export interface PageHeaderStatus {
  summary: string;
  /** The fuller wording, for the tooltip and screen readers. */
  label: string;
  /** Percent done; null when nothing is in progress. */
  percent: number | null;
  tone: "info" | "success" | "caution" | "warning";
}

interface PageHeaderProps {
  title: ComponentChildren;
  /** A small icon before the title saying what kind of thing the page is
   *  (an account's kind); it carries its own tooltip. */
  titleIcon?: ComponentChildren;
  back?: PageHeaderBack;
  actions?: ComponentChildren;
  status?: PageHeaderStatus | null;
}

/**
 * The one page header (UI consistency contract): where you are on the left
 * (back control and title together), what is happening and what you can do
 * on the right (status, then actions), at a fixed height so nothing
 * shifts between pages. Status never changes that height: its bar sits on
 * the bottom border.
 */
export function PageHeader({ title, titleIcon, back, actions, status }: PageHeaderProps) {
  return (
    <div class="page-chrome">
      <header class="page-header">
        <div class="page-header-start">
          {back && (
            <a
              class="page-header-back-link"
              href={back.href}
              onClick={(event) => {
                event.preventDefault();
                back.onBack();
              }}
              aria-label={back.label}
              title={back.label}
            >
              <IconChevronLeft size={24} stroke={2.2} />
            </a>
          )}
          {titleIcon && <span class="page-header-title-icon">{titleIcon}</span>}
          <h2 class="page-header-title">{title}</h2>
        </div>
        <div class="page-header-end">
          {status && (
            <span class="page-header-status" title={status.label} aria-label={status.label}>
              {status.percent === null && <span class={`page-header-status-dot is-${status.tone}`} aria-hidden="true" />}
              <span class="page-header-status-text">{status.summary}</span>
            </span>
          )}
          {actions}
        </div>
      </header>
      {status && status.percent !== null && (
        <div
          class="page-header-progress"
          role="progressbar"
          aria-label={status.label}
          aria-valuenow={Math.round(status.percent)}
          aria-valuemin={0}
          aria-valuemax={100}
        >
          <div class="page-header-progress-fill" style={{ width: `${status.percent.toFixed(1)}%` }} />
        </div>
      )}
    </div>
  );
}
