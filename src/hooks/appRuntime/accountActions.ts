import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import type {
  ConflictResolution,
  SyncProblemsView,
  AccountKind,
  AccountProfile,
  CreateAccountProfileInput,
  RenameAccountProfileInput,
  SetAccountAgentStateInput,
  SetAccountSyncRootInput,
  SyncAgentState,
  ToastType,
} from "../../types/somedrive";

interface AccountActionsFactoryParams {
  showToast: (message: string, type?: ToastType) => void;
  refreshStatus: () => Promise<void>;
  refreshActivity: () => Promise<void>;
  openAccount: (accountId: string) => void;
}

export function createAccountActions({
  showToast,
  refreshStatus,
  refreshActivity,
  openAccount,
}: AccountActionsFactoryParams) {
  const refreshAll = () => Promise.all([refreshStatus(), refreshActivity()]);

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
      await refreshAll();
      const authStarted = await startInteractiveAuth(profile.id);
      openAccount(profile.id);
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
      await refreshAll();
    } catch (error) {
      showToast(`Failed to rename account: ${error}`, "error");
    }
  };

  const removeAccountProfile = async (id: string) => {
    try {
      await invoke("remove_account_profile", { input: { id } });
      showToast("Account removed. Synced files were not deleted.", "info");
      await refreshAll();
    } catch (error) {
      showToast(`Failed to remove account: ${error}`, "error");
    }
  };

  const setAccountAgentState = async (id: string, agentState: SyncAgentState) => {
    const input: SetAccountAgentStateInput = { id, agentState };
    try {
      await invoke("set_account_agent_state", { input });
      showToast(`Agent state set to '${agentState}'.`, "success");
      await refreshAll();
    } catch (error) {
      showToast(`Failed to update agent state: ${error}`, "error");
    }
  };

  const setAccountSyncRoot = async (id: string, syncRoot: string) => {
    const input: SetAccountSyncRootInput = { id, syncRoot };
    try {
      await invoke("set_account_sync_root", { input });
      showToast("Sync folder updated.", "success");
      await refreshAll();
    } catch (error) {
      showToast(`Failed to update sync folder: ${error}`, "error");
    }
  };

  const clearAccountAuth = async (profileId: string) => {
    try {
      await invoke("clear_account_auth", { profileId });
      showToast("Account auth session cleared.", "info");
      await refreshAll();
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
      await refreshAll();
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
      await refreshAll();
    } catch (error) {
      showToast(`Failed to resume all accounts: ${error}`, "error");
    }
  };

  const retryAccountSync = async (profileId: string) => {
    const input: SetAccountAgentStateInput = { id: profileId, agentState: "syncing" };
    try {
      await invoke("set_account_agent_state", { input });
      await refreshAll();
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
      await refreshAll();
    } catch (error) {
      showToast(`Failed to retry download: ${error}`, "error");
    }
  };

  const fetchSyncProblems = async (profileId: string): Promise<SyncProblemsView> => {
    try {
      return await invoke<SyncProblemsView>("list_sync_problems", { profileId });
    } catch (error) {
      showToast(`Failed to read sync problems: ${error}`, "error");
      return { problems: [], notSynced: [] };
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
      await refreshAll();
    } catch (error) {
      showToast(`Failed to retry downloads: ${error}`, "error");
    }
  };

  const confirmAccountLargeDelete = async (profileId: string) => {
    try {
      await invoke("confirm_account_large_delete", { profileId });
      showToast("Large deletion confirmed. Cloud changes will apply on the next sync cycle.", "info");
      await refreshAll();
    } catch (error) {
      showToast(`Failed to confirm deletion: ${error}`, "error");
    }
  };

  const keepCloudFilesAfterLargeDelete = async (profileId: string) => {
    try {
      await invoke("keep_cloud_files_after_large_delete", { profileId });
      showToast("Keeping cloud files. Initial sync will restore missing items locally.", "info");
      await refreshAll();
    } catch (error) {
      showToast(`Failed to keep cloud files: ${error}`, "error");
    }
  };

  const fetchAccountLargeDeletePreview = async (profileId: string) => {
    try {
      return await invoke<string[]>("get_account_large_delete_preview", { profileId });
    } catch (error) {
      showToast(`Failed to load deletion preview: ${error}`, "error");
      return [];
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
    fetchSyncProblems,
    resolveSyncConflicts,
    confirmAccountLargeDelete,
    keepCloudFilesAfterLargeDelete,
    fetchAccountLargeDeletePreview,
    exportAccountLargeDeletePreview,
  };
}
