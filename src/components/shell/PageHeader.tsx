import { IconChevronLeft } from "@tabler/icons-preact";
import type { ComponentChildren } from "preact";

interface PageHeaderBack {
  href: string;
  label: string;
  onBack: () => void;
}

interface PageHeaderProps {
  title: ComponentChildren;
  back?: PageHeaderBack;
  actions?: ComponentChildren;
}

/**
 * The one page header (UI consistency contract): back control left, title
 * centred, actions right, at a fixed height so nothing shifts between pages.
 */
export function PageHeader({ title, back, actions }: PageHeaderProps) {
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
              <IconChevronLeft size={26} stroke={2.2} />
            </a>
          )}
        </div>
        <h2 class="page-header-title">{title}</h2>
        <div class="page-header-end">{actions}</div>
      </header>
    </div>
  );
}
