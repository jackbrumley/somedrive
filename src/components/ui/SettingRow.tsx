import type { ComponentChildren } from "preact";

interface SettingRowProps {
  label: ComponentChildren;
  /** Id of the control the label names, when there is a single one. */
  labelFor?: string;
  description?: ComponentChildren;
  children?: ComponentChildren;
}

/** One setting: label and description on the left, its controls on the right. */
export function SettingRow({ label, labelFor, description, children }: SettingRowProps) {
  return (
    <div class="setting-row">
      <div class="setting-row-copy">
        {labelFor ? (
          <label class="setting-row-label" for={labelFor}>
            {label}
          </label>
        ) : (
          <span class="setting-row-label">{label}</span>
        )}
        {description ? <span class="setting-row-description">{description}</span> : null}
      </div>
      {children ? <div class="setting-row-controls">{children}</div> : null}
    </div>
  );
}
