import { IconFolderOpen, IconLogin2, IconLogout, IconTrash } from "@tabler/icons-preact";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "preact/hooks";
import type { AccountProfile } from "../../types/somedrive";
import { SettingRow } from "../ui/SettingRow";

interface AccountSettingsPanelProps {
  account: AccountProfile;
  onStartAuth: (accountId: string) => Promise<unknown>;
  onRename: (id: string, name: string) => Promise<void>;
  onSetSyncRoot: (id: string, path: string) => Promise<void>;
  onClearAuth: (id: string) => Promise<void>;
  onRemoveProfile: (id: string) => Promise<void>;
  onOpenSyncRootFolder: (accountId: string) => Promise<void>;
}

function formatLastSync(lastSyncAt: string | null | undefined): string {
  return lastSyncAt ? new Date(lastSyncAt).toLocaleString() : "Never";
}

/** Account configuration: name, sync folder, Microsoft sign-in and removal. */
export function AccountSettingsPanel({
  account,
  onStartAuth,
  onRename,
  onSetSyncRoot,
  onClearAuth,
  onRemoveProfile,
  onOpenSyncRootFolder,
}: AccountSettingsPanelProps) {
  const [draftName, setDraftName] = useState(account.displayName);
  const trimmedDraftName = draftName.trim();
  const nameChanged = trimmedDraftName !== "" && trimmedDraftName !== account.displayName;
  const accountKindLabel = account.kind === "business" ? "Work or school account" : "Personal account";

  useEffect(() => {
    setDraftName(account.displayName);
  }, [account.displayName]);

  const chooseSyncFolder = async () => {
    const selected = await open({
      directory: true,
      defaultPath: account.syncRoot,
      title: `Choose sync folder for ${account.displayName}`,
    });
    if (typeof selected !== "string" || !selected.trim()) {
      return;
    }
    if (/\/OneDrive$/i.test(selected.replace(/\/+$/, ""))) {
      const confirmed = window.confirm(
        "This looks like the default folder used by other OneDrive apps. It is safer to use a SomeDrive folder to avoid conflicts. Continue anyway?"
      );
      if (!confirmed) {
        return;
      }
    }
    await onSetSyncRoot(account.id, selected);
  };

  const signOut = () => {
    const confirmed = window.confirm(
      `Sign ${account.email || account.displayName} out of SomeDrive? Syncing stops until you sign in again. Files on this device are kept.`
    );
    if (confirmed) {
      void onClearAuth(account.id);
    }
  };

  const removeAccount = () => {
    const confirmed = window.confirm(
      `Remove ${account.displayName} from SomeDrive? Syncing stops and the account is forgotten. Files in ${account.syncRoot} and in OneDrive are not deleted.`
    );
    if (confirmed) {
      void onRemoveProfile(account.id);
    }
  };

  return (
    <div class="account-settings">
      <article class="card">
        <h3>Account</h3>
        <dl class="facts-grid">
          <div>
            <dt>Microsoft account</dt>
            <dd>{account.email || "Not signed in"}</dd>
          </div>
          <div>
            <dt>Type</dt>
            <dd>{accountKindLabel}</dd>
          </div>
          <div>
            <dt>Last sync</dt>
            <dd>{formatLastSync(account.lastSyncAt)}</dd>
          </div>
        </dl>
        <div class="settings-list">
          <form
            class="setting-row"
            onSubmit={(event) => {
              event.preventDefault();
              if (nameChanged) {
                void onRename(account.id, trimmedDraftName);
              }
            }}
          >
            <div class="setting-row-copy">
              <label class="setting-row-label" htmlFor="account-display-name">
                Display name
              </label>
              <span class="setting-row-description">How this account appears in SomeDrive.</span>
            </div>
            <div class="setting-row-controls">
              <input
                id="account-display-name"
                value={draftName}
                onInput={(event) => setDraftName(event.currentTarget.value)}
              />
              <button type="submit" class="ghost" disabled={!nameChanged}>
                Save
              </button>
            </div>
          </form>
        </div>
      </article>

      <article class="card">
        <h3>Sync folder</h3>
        <div class="settings-list">
          <SettingRow
            label="Location on this device"
            description={<span class="setting-row-value">{account.syncRoot}</span>}
          >
            <button type="button" class="ghost" onClick={() => void onOpenSyncRootFolder(account.id)}>
              <IconFolderOpen size={15} />
              Open
            </button>
            <button type="button" class="ghost" onClick={() => void chooseSyncFolder()}>
              Change…
            </button>
          </SettingRow>
        </div>
      </article>

      <article class="card">
        <h3>Microsoft sign-in</h3>
        <div class="settings-list">
          <SettingRow
            label={account.authConfigured ? "Signed in" : "Not signed in"}
            description={
              account.authConfigured
                ? `Syncing as ${account.email}. Sign in again if Microsoft asks you to, or to switch account.`
                : "Sign in to Microsoft to start syncing this account."
            }
          >
            <button
              type="button"
              class={account.authConfigured ? "ghost" : undefined}
              onClick={() => void onStartAuth(account.id)}
            >
              <IconLogin2 size={15} />
              {account.authConfigured ? "Sign in again" : "Sign in"}
            </button>
            {account.authConfigured && (
              <button type="button" class="ghost" onClick={signOut}>
                <IconLogout size={15} />
                Sign out
              </button>
            )}
          </SettingRow>
        </div>
      </article>

      <article class="card account-settings-danger">
        <h3>Remove account</h3>
        <div class="settings-list">
          <SettingRow
            label="Remove from SomeDrive"
            description="Stops syncing and forgets this account. Your files on this device and in OneDrive are kept."
          >
            <button type="button" class="danger" onClick={removeAccount}>
              <IconTrash size={15} />
              Remove
            </button>
          </SettingRow>
        </div>
      </article>
    </div>
  );
}
