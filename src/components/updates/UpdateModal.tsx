import type { UpdateCheckResult, UpdateInstallStage } from "../../types/somedrive";
import { Modal } from "../ui/Modal";

interface UpdateModalProps {
  updateResult: UpdateCheckResult;
  installStage: UpdateInstallStage;
  installError: string | null;
  onUpdateNow: () => void;
  onViewReleaseNotes: () => void;
  onLater: () => void;
}

const installStageMessages: Record<Exclude<UpdateInstallStage, "idle">, string> = {
  finishing_sync: "Finishing sync before updating… Transfers in progress are completed or safely paused first.",
  starting: "Starting update… SomeDrive will restart shortly.",
};

export function UpdateModal({
  updateResult,
  installStage,
  installError,
  onUpdateNow,
  onViewReleaseNotes,
  onLater,
}: UpdateModalProps) {
  const installing = installStage !== "idle";

  return (
    <Modal title="Update Available" onClose={onLater} dismissible={!installing}>
      <div class="update-modal">
        <p class="update-modal-versions">
          <span>v{updateResult.currentVersion}</span>
          <span aria-hidden="true">→</span>
          <strong>v{updateResult.latestVersion}</strong>
        </p>
        <p class="update-modal-summary">
          A new version of SomeDrive is available. Updating keeps your accounts and sync settings, and syncing
          accounts resume after the restart.
        </p>
        {installing && (
          <p class="update-modal-status" aria-live="polite">
            {installStageMessages[installStage]}
          </p>
        )}
        {installError && <p class="error">Update could not start: {installError}</p>}
        <div class="button-row">
          <button type="button" onClick={onUpdateNow} disabled={installing}>
            {installing ? "Updating…" : "Update Now"}
          </button>
          <button type="button" class="ghost" onClick={onViewReleaseNotes}>
            View Release Notes
          </button>
          <button type="button" class="ghost" onClick={onLater} disabled={installing}>
            Later
          </button>
        </div>
      </div>
    </Modal>
  );
}
