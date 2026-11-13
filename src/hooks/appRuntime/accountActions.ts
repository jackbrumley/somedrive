import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import type {
  AccountRemovalOutcome,
  ConflictResolution,
  AccountKind,
  AccountProfile,
  CreateAccountProfileInput,
  RemoveAccountProfileInput,
  RenameAccountProfileInput,
  SetAccountAgentStateInput,
  SetAccountSyncRootInput,
  SyncAgentState,
  ToastType,
} from "../../types/somedrive";

interface AccountActionsFactoryParams {
  showToast: (message: string, type?: ToastType) => void;
  /** Opens the account page once the store lists the account. */
  openAccountWhenListed: (accountId: string) => void;
}

/** Account actions. Their effect reaches the window through the store
 *  (the backend's change counters), so none of them reloads anything. */
export function createAccountActions({ showToast, openAccountWhenListed }: AccountActionsFactoryParams) {

  const startInteractiveAuth = async (profileId: string) => {
    try {
      await invoke("start_interactive_auth", { profileId });
      showToast("Microsoft sign-in opened. Complete sign-in in the auth window.", "info");
      return true;
    } catch (error) {
      showToast(`Failed to start sign-in: ${error}`, "error");
      return false;
    }
  };

  const createAccountProfile = async (displayName: string, kind: AccountKind) => {
    const input: CreateAccountProfileInput = { displayName, kind };
    try {
      const profile = await invoke<AccountProfile>("create_account_profile", { input });
      showToast(`Account '${displayName}' added.`, "success");
      const authStarted = await startInteractiveAuth(profile.id);
      openAccountWhenListed(profile.id);
      return authStarted;
    } catch (error) {
      showToast(`Failed to add account: ${error}`, "error");
      return false;
    }
  };

  const renameAccountProfile = async (id: string, displayName: string) => {
    const input: RenameAccountProfileInput = { id, displayName };
    try {
      await invoke("rename_account_profile", { input });
      showToast("Account name updated.", "success");
    } catch (error) {
      showToast(`Failed to rename account: ${error}`, "error");
    }
  };

  /** Rejects with the reason when the account was not removed (the dialog shows it). */
  const removeAccountProfile = async (id: string, moveLocalFolderToTrash: boolean) => {
    const input: RemoveAccountProfileInput = { id, moveLocalFolderToTrash };
    const outcome = await invoke<AccountRemovalOutcome>("remove_account_profile", { input });
    showToast(
      outcome.localFolderTrashed
        ? "Account removed. Its sync folder is in the Trash; your files in OneDrive are kept."
        : "Account removed. Its sync folder and your files in OneDrive are kept.",
      "info"
    );
    for (const warning of outcome.warnings) {
      showToast(`Account removed, but: ${warning}`, "error");
    }
  };

  const setAccountAgentState = async (id: string, agentState: SyncAgentState) => {
    const input: SetAccountAgentStateInput = { id, agentState };
    try {
      await invoke("set_account_agent_state", { input });
      showToast(`Agent state set to '${agentState}'.`, "success");
    } catch (error) {
      showToast(`Failed to update agent state: ${error}`, "error");
    }
  };

  const setAccountSyncRoot = async (id: string, syncRoot: string) => {
    const input: SetAccountSyncRootInput = { id, syncRoot };
    try {
      await invoke("set_account_sync_root", { input });
      showToast("Sync folder updated.", "success");
    } catch (error) {
      showToast(`Failed to update sync folder: ${error}`, "error");
    }
  };

  const clearAccountAuth = async (profileId: string) => {
    try {
      await invoke("clear_account_auth", { profileId });
      showToast("Account auth session cleared.", "info");
    } catch (error) {
      showToast(`Failed to clear auth: ${error}`, "error");
    }
  };

  const pauseAllAccounts = async () => {
    try {
      const pausedCount = await invoke<number>("pause_all_accounts");
      if (pausedCount > 0) {
        showToast(`Paused synchronization for ${pausedCount} account${pausedCount === 1 ? "" : "s"}.`, "info");
      } else {
        showToast("No syncing accounts to pause.", "info");
      }
    } catch (error) {
      showToast(`Failed to pause all accounts: ${error}`, "error");
    }
  };

  const resumeAllAccounts = async () => {
    try {
      const resumedCount = await invoke<number>("resume_all_accounts");
      if (resumedCount > 0) {
        showToast(`Resumed synchronization for ${resumedCount} account${resumedCount === 1 ? "" : "s"}.`, "success");
      } else {
        showToast("No paused accounts to resume.", "info");
      }
    } catch (error) {
      showToast(`Failed to resume all accounts: ${error}`, "error");
    }
  };

  const retryAccountSync = async (profileId: string) => {
    const input: SetAccountAgentStateInput = { id: profileId, agentState: "syncing" };
    try {
      await invoke("set_account_agent_state", { input });
    } catch (error) {
      showToast(`Failed to retry sync: ${error}`, "error");
    }
  };

  const retryFailedDownload = async (profileId: string, recentItemId: string, path: string) => {
    try {
      const response = await invoke<{ status: "retried" | "already_retrying" }>(
        "retry_failed_download",
        { profileId, recentItemId }
      );
      if (response.status === "retried") {
        showToast(`Retry queued for '${path}'.`, "info");
      } else {
        showToast(`'${path}' is already queued or no longer failed.`, "info");
      }
    } catch (error) {
      showToast(`Failed to retry download: ${error}`, "error");
    }
  };

  const resolveSyncConflicts = async (profileId: string, paths: string[], resolution: ConflictResolution) => {
    try {
      const recorded = await invoke<number>("resolve_sync_conflicts", { profileId, paths, resolution });
      showToast(`${recorded} file${recorded === 1 ? "" : "s"} will be resolved on the next sync.`, "success");
    } catch (error) {
      showToast(`Failed to resolve: ${error}`, "error");
    }
  };

  const retryAllFailedDownloads = async (profileId: string) => {
    try {
      const response = await invoke<{
        retried: number;
        skippedPermissionDenied: number;
        alreadyRetrying: number;
      }>("retry_all_failed_downloads", { profileId });
      if (response.retried > 0) {
        showToast(
          `Retry queued for ${response.retried} failed download${response.retried === 1 ? "" : "s"}.`,
          "info"
        );
      } else if (response.alreadyRetrying > 0) {
        showToast(
          `${response.alreadyRetrying} file${response.alreadyRetrying === 1 ? " is" : "s are"} already queued or no longer failed.`,
          "info"
        );
      } else {
        showToast("No failed downloads found.", "info");
      }
    } catch (error) {
      showToast(`Failed to retry downloads: ${error}`, "error");
    }
  };

  const confirmAccountLargeDelete = async (profileId: string) => {
    try {
      await invoke("confirm_account_large_delete", { profileId });
      showToast("Large deletion confirmed. Cloud changes will apply on the next sync cycle.", "info");
    } catch (error) {
      showToast(`Failed to confirm deletion: ${error}`, "error");
    }
  };

  const keepCloudFilesAfterLargeDelete = async (profileId: string) => {
    try {
      await invoke("keep_cloud_files_after_large_delete", { profileId });
      showToast("Keeping cloud files. Initial sync will restore missing items locally.", "info");
    } catch (error) {
      showToast(`Failed to keep cloud files: ${error}`, "error");
    }
  };

  const exportAccountLargeDeletePreview = async (profileId: string) => {
    try {
      const defaultFileName = `somedrive-large-delete-review-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-")}.txt`;
      const destinationPath = await save({
        title: "Export large deletion review",
        defaultPath: defaultFileName,
      });
      if (typeof destinationPath !== "string" || !destinationPath.trim()) {
        return;
      }
      const writtenPath = await invoke<string>("export_account_large_delete_preview", {
        profileId,
        destinationPath,
      });
      showToast(`Deletion review exported to ${writtenPath}.`, "success");
    } catch (error) {
      showToast(`Failed to export deletion review: ${error}`, "error");
    }
  };

  return {
    createAccountProfile,
    renameAccountProfile,
    removeAccountProfile,
    setAccountAgentState,
    setAccountSyncRoot,
    startInteractiveAuth,
    clearAccountAuth,
    pauseAllAccounts,
    resumeAllAccounts,
    retryAccountSync,
    retryFailedDownload,
    retryAllFailedDownloads,
    resolveSyncConflicts,
    confirmAccountLargeDelete,
    keepCloudFilesAfterLargeDelete,
    exportAccountLargeDeletePreview,
  };
}
