import { invoke } from "@tauri-apps/api/core";
import type {
  ActivityEvent,
  AppStatusSnapshot,
  SyncRuntimeSnapshot,
  ToastType,
} from "../../types/somedrive";

interface RefreshFactoryParams {
  showToast: (message: string, type?: ToastType, durationMs?: number) => void;
  setStatus: (value: AppStatusSnapshot) => void;
  setActivityEvents: (value: ActivityEvent[]) => void;
  setSyncRuntime: (value: SyncRuntimeSnapshot | ((current: SyncRuntimeSnapshot) => SyncRuntimeSnapshot)) => void;
}

export function createRefreshActions({
  showToast,
  setStatus,
  setActivityEvents,
  setSyncRuntime,
}: RefreshFactoryParams) {
  const snapshotTime = (value: string): number => {
    const parsed = Date.parse(value);
    return Number.isFinite(parsed) ? parsed : 0;
  };

  const shouldReplaceSnapshot = (current: SyncRuntimeSnapshot, next: SyncRuntimeSnapshot): boolean => {
    if (next.revision > current.revision) {
      return true;
    }
    if (next.revision < current.revision) {
      return false;
    }
    return snapshotTime(next.generatedAt) > snapshotTime(current.generatedAt);
  };

  const refreshStatus = async () => {
    try {
      const snapshot = await invoke<AppStatusSnapshot>("get_status_snapshot");
      setStatus(snapshot);
    } catch (error) {
      showToast(`Failed to load status: ${error}`, "error");
    }
  };

  const refreshActivity = async () => {
    try {
      const events = await invoke<ActivityEvent[]>("list_activity_events", { limit: 200 });
      setActivityEvents(events);
    } catch (error) {
      showToast(`Failed to load activity: ${error}`, "error");
    }
  };

  const refreshSyncRuntime = async () => {
    try {
      const snapshot = await invoke<SyncRuntimeSnapshot>("get_sync_runtime_snapshot");
      setSyncRuntime((current) => (shouldReplaceSnapshot(current, snapshot) ? snapshot : current));
    } catch (error) {
      showToast(`Failed to load sync runtime: ${error}`, "error", 4200);
      throw error;
    }
  };

  return {
    refreshStatus,
    refreshActivity,
    refreshSyncRuntime,
  };
}
