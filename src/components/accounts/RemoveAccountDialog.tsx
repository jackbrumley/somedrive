import { IconAlertTriangle, IconShieldCheck, IconTrash } from "@tabler/icons-preact";
import { useState } from "preact/hooks";
import { useAccountAreas } from "../../state/accountAreas";
import type { AccountProfile } from "../../types/somedrive";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { ToggleSwitch } from "../ui/ToggleSwitch";

interface RemoveAccountDialogProps {
  account: AccountProfile;
  /** The account's activity change counter: the folder check reruns when it moves. */
  activityRevision: number | undefined;
  onCopyText: (text: string) => Promise<boolean>;
  /** Rejects with the reason when the account was not removed. */
  onRemove: (moveLocalFolderToTrash: boolean) => Promise<void>;
  onClose: () => void;
}

/**
 * Confirms removing an account: the user types the account's email (or its
 * name, before it has signed in). The sync folder is kept unless the user
 * chooses the Trash, which the backend allows only when every file in it has
 * reached OneDrive and the folder holds only this account's files.
 */
export function RemoveAccountDialog({
  account,
  activityRevision,
  onCopyText,
  onRemove,
  onClose,
}: RemoveAccountDialogProps) {
  const [moveToTrash, setMoveToTrash] = useState(false);
  const { data: preview, error: previewError } = useAccountAreas().useRemovalPreview(
    account.id,
    activityRevision,
    true
  );
  const trashAllowed = preview !== null && preview.localFolderBlockedReason === null;
  const trashChosen = moveToTrash && trashAllowed;
  const confirmWord = account.email.trim() || account.displayName;

  const trashDescription = previewError
    ? `Couldn't check the sync folder: ${previewError}`
    : preview === null
      ? "Checking the sync folder…"
      : (preview.localFolderBlockedReason ?? `${preview.syncRoot}. You can restore it from the Trash.`);

  return (
    <ConfirmDialog
      title={`Remove ${account.displayName}?`}
      summary={
        <>
          <p>
            SomeDrive stops syncing this account and forgets it, including its Microsoft sign-in on this device.
          </p>
          <p class="confirm-dialog-safe">
            <IconShieldCheck size={16} />
            <span>Your files in OneDrive are not touched.</span>
          </p>
        </>
      }
      options={
        <>
          <ToggleSwitch
            id="remove-account-trash-folder"
            label="Also move the sync folder to the Trash"
            description={trashDescription}
            checked={trashChosen}
            disabled={!trashAllowed}
            onChange={setMoveToTrash}
          />
          {trashChosen && (
            <p class="confirm-dialog-warning">
              <IconAlertTriangle size={16} />
              <span>
                If sync was paused, files you added since then are not in OneDrive. Check the folder in the Trash
                before you empty it.
              </span>
            </p>
          )}
        </>
      }
      confirmWord={confirmWord}
      onCopyConfirmWord={onCopyText}
      confirmLabel={trashChosen ? "Remove and move folder to Trash" : "Remove account"}
      confirmIcon={<IconTrash size={15} />}
      danger
      busyText="Stopping sync and removing the account…"
      onConfirm={() => onRemove(trashChosen)}
      onClose={onClose}
    />
  );
}
