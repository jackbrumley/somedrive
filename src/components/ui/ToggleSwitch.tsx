interface ToggleSwitchProps {
  id: string;
  label: string;
  description?: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void | Promise<void>;
}

export function ToggleSwitch({
  id,
  label,
  description,
  checked,
  disabled = false,
  onChange,
}: ToggleSwitchProps) {
  const handleChange = (event: Event) => {
    const nextChecked = (event.currentTarget as HTMLInputElement).checked;
    void onChange(nextChecked);
  };

  return (
    <label class={`setting-row toggle-field${disabled ? " is-disabled" : ""}`} for={id}>
      <span class="setting-row-copy">
        <span class="setting-row-label">{label}</span>
        {description ? <span class="setting-row-description">{description}</span> : null}
      </span>
      <span class="toggle-control">
        <input
          id={id}
          class="toggle-input"
          type="checkbox"
          checked={checked}
          disabled={disabled}
          onChange={handleChange}
        />
        <span class="toggle-track" aria-hidden="true">
          <span class="toggle-thumb" />
        </span>
      </span>
    </label>
  );
}
