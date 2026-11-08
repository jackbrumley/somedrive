import { useState } from "preact/hooks";
import type { AccountKind } from "../../types/somedrive";
import { Modal } from "../ui/Modal";
import { SelectField, type SelectFieldOption } from "../ui/SelectField";

interface AddAccountModalProps {
  onClose: () => void;
  onCreateAccount: (displayName: string, kind: AccountKind) => Promise<boolean>;
}

const accountKindOptions: SelectFieldOption[] = [
  { value: "personal", label: "Personal" },
  { value: "business", label: "Business" },
];

export function AddAccountModal({ onClose, onCreateAccount }: AddAccountModalProps) {
  const [name, setName] = useState("");
  const [kind, setKind] = useState<AccountKind>("personal");
  const [saving, setSaving] = useState(false);

  const submit = async (event: Event) => {
    event.preventDefault();
    const trimmed = name.trim();
    if (!trimmed || saving) {
      return;
    }
    setSaving(true);
    try {
      const created = await onCreateAccount(trimmed, kind);
      if (created) {
        onClose();
      }
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal title="Add Account" onClose={onClose} dismissible={!saving}>
      <form class="account-form" onSubmit={submit}>
        <label class="field-label" for="new-account-name">
          Account Name
        </label>
        <input
          id="new-account-name"
          value={name}
          onInput={(event) => setName(event.currentTarget.value)}
          placeholder="Personal, Work, Family"
          autoFocus
        />

        <label class="field-label" for="new-account-kind">
          Account Type
        </label>
        <SelectField
          id="new-account-kind"
          name="new-account-kind"
          value={kind}
          options={accountKindOptions}
          onValueChange={(next) => setKind(next as AccountKind)}
        />

        <div class="button-row">
          <button type="submit" disabled={!name.trim() || saving}>
            {saving ? "Adding..." : "Add Account"}
          </button>
          <button type="button" class="ghost" onClick={onClose} disabled={saving}>
            Cancel
          </button>
        </div>
      </form>
    </Modal>
  );
}
