import { invoke } from "@tauri-apps/api/core";
import { formatBytes } from "../../components/accounts/syncFormatting";
import type { CleanupActions, CleanupDeleteResult, ToastType } from "../../types/somedrive";

interface CleanupActionsFactoryParams {
  showToast: (message: string, type?: ToastType) => void;
}

/** The Clean up tab's actions. Deleting rejects with the reason, which the
 *  confirmation dialog shows in place. The list itself comes from the store
 *  and updates by itself after a delete or hide. */
export function createCleanupActions({ showToast }: CleanupActionsFactoryParams): CleanupActions {
  const announceDeleted = (result: CleanupDeleteResult) => {
    const files = `${result.fileCount} file${result.fileCount === 1 ? "" : "s"}`;
    showToast(
      `Deleted ${files} (${formatBytes(result.bytes)}). They're in the Trash, and OneDrive keeps them in its recycle bin.`,
      "success"
    );
  };

  return {
    deleteDuplicates: async (accountId: string, suggestionId: string) => {
      const result = await invoke<CleanupDeleteResult>("delete_cleanup_duplicates", {
        profileId: accountId,
        suggestionId,
      });
      announceDeleted(result);
      return result;
    },
    deleteLargeFile: async (accountId: string, path: string) => {
      const result = await invoke<CleanupDeleteResult>("delete_cleanup_large_file", { profileId: accountId, path });
      announceDeleted(result);
      return result;
    },
    hide: async (accountId: string, path: string) => {
      try {
        await invoke("hide_cleanup_path", { profileId: accountId, path });
      } catch (error) {
        showToast(`Failed to hide '${path}': ${error}`, "error");
      }
    },
    unhide: async (accountId: string, path: string) => {
      try {
        await invoke("unhide_cleanup_path", { profileId: accountId, path });
      } catch (error) {
        showToast(`Failed to show '${path}' again: ${error}`, "error");
      }
    },
    copyText: async (text: string) => {
      try {
        await invoke("copy_text_to_clipboard", { text });
        return true;
      } catch (error) {
        showToast(`Failed to copy to the clipboard: ${error}`, "error");
        return false;
      }
    },
    openFile: async (accountId: string, path: string) => {
      try {
        await invoke("open_account_item", { input: { profileId: accountId, relativePath: path } });
      } catch (error) {
        showToast(`Failed to open '${path}': ${error}`, "error");
      }
    },
  };
}
