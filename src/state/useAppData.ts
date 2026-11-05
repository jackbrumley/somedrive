import { listen } from "@tauri-apps/api/event";
import { disable as disableAutostart, enable as enableAutostart } from "@tauri-apps/plugin-autostart";
import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import type {
  AppDataChangedEvent,
  AppStatusSnapshot,
  SyncRuntimeSnapshot,
  SyncStatusEvent,
  SystemInfo,
  ToastType,
} from "../types/somedrive";
import {
  readSessionLogText,
  readSettings,
  readStatusSnapshot,
  readSyncRuntime,
  readSystemInfo,
  requestSyncStatusSnapshot,
  type AppSettingsView,
} from "./backendReads";
import { coalesced } from "./coalesce";

// The app-wide part of the store (docs/ui-data-flow.md):
// - each account's live status arrives as numbered `sync-status` events;
//   a gap in the numbers rereads the whole snapshot;
// - the account list and settings are reread on `app-data-changed`, which
//   the backend sends after every save of a file the window shows.
// Screens read from here and never fetch.

const initialStatus: AppStatusSnapshot = {
  appVersion: "0.0.0",
  platform: "unknown",
  syncEngineReady: false,
  authConfigured: false,
  activeAccount: null,
  lastSyncAt: null,
  health: "degraded",
  accounts: [],
};

const initialSyncRuntime: SyncRuntimeSnapshot = {
  generatedAt: new Date(0).toISOString(),
  revision: 0,
  accounts: [],
};

/** How long to wait for the first `sync-status` events before reading the
 *  snapshot directly. */
const FIRST_STATUS_WAIT_MS = 1200;

function snapshotTime(value: string): number {
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

function isNewerSnapshot(current: SyncRuntimeSnapshot, next: SyncRuntimeSnapshot): boolean {
  if (next.revision !== current.revision) {
    return next.revision > current.revision;
  }
  return snapshotTime(next.generatedAt) > snapshotTime(current.generatedAt);
}

interface UseAppDataProps {
  showToast: (message: string, type?: ToastType) => void;
}

export function useAppData({ showToast }: UseAppDataProps) {
  const [status, setStatus] = useState<AppStatusSnapshot>(initialStatus);
  const [syncRuntime, setSyncRuntime] = useState<SyncRuntimeSnapshot>(initialSyncRuntime);
  const [settings, setSettings] = useState<AppSettingsView | null>(null);
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null);
  const statusSeqByAccount = useRef<Record<string, number>>({});
  const hasReceivedSyncStatus = useRef(false);
  const showToastRef = useRef(showToast);
  showToastRef.current = showToast;

  const reloaders = useMemo(() => {
    const report = (what: string) => (error: unknown) => showToastRef.current(`Failed to load ${what}: ${error}`, "error");
    return {
      appData: coalesced(async () => {
        await Promise.all([
          readStatusSnapshot().then(setStatus, report("accounts")),
          readSettings().then(setSettings, report("settings")),
        ]);
      }),
      syncRuntime: coalesced(async () => {
        try {
          const snapshot = await readSyncRuntime();
          setSyncRuntime((current) => (isNewerSnapshot(current, snapshot) ? snapshot : current));
        } catch (error) {
          report("sync status")(error);
        }
      }),
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let firstStatusTimer: number | null = null;

    const unlistenAppData = listen<AppDataChangedEvent>("app-data-changed", () => {
      if (!disposed) {
        reloaders.appData();
      }
    });

    const unlistenSyncStatus = listen<SyncStatusEvent>("sync-status", ({ payload }) => {
      if (disposed || !payload?.profileId) {
        return;
      }
      hasReceivedSyncStatus.current = true;
      const profileId = payload.profileId;
      const incomingSeq = payload.statusSeq ?? 0;
      const currentSeq = statusSeqByAccount.current[profileId] ?? 0;
      if (incomingSeq <= 0 || incomingSeq <= currentSeq) {
        return;
      }
      if (currentSeq > 0 && incomingSeq > currentSeq + 1) {
        reloaders.syncRuntime();
      }
      statusSeqByAccount.current = { ...statusSeqByAccount.current, [profileId]: incomingSeq };
      setSyncRuntime((current) => {
        const accounts = current.accounts.filter((account) => account.profileId !== profileId);
        if (payload.kind !== "removed" && payload.status) {
          accounts.push(payload.status);
          accounts.sort((left, right) => left.profileId.localeCompare(right.profileId));
        }
        return {
          generatedAt: payload.generatedAt || new Date().toISOString(),
          revision: Math.max(current.revision + 1, incomingSeq),
          accounts,
        };
      });
    });

    // Listen first, then read, so nothing saved in between is missed.
    void unlistenAppData.then(() => {
      if (!disposed) {
        reloaders.appData();
      }
    });
    void unlistenSyncStatus
      .then(() => (disposed ? undefined : requestSyncStatusSnapshot()))
      .then(() => {
        if (disposed) {
          return;
        }
        firstStatusTimer = window.setTimeout(() => {
          if (!disposed && !hasReceivedSyncStatus.current) {
            reloaders.syncRuntime();
          }
        }, FIRST_STATUS_WAIT_MS);
      })
      .catch((error) => {
        if (!disposed) {
          showToastRef.current(`Failed to load sync status: ${error}`, "error");
        }
      });
    readSystemInfo().then(setSystemInfo, (error) =>
      showToastRef.current(`Failed to read system info: ${error}`, "error")
    );

    return () => {
      disposed = true;
      for (const unlisten of [unlistenAppData, unlistenSyncStatus]) {
        unlisten
          .then((stop) => stop())
          .catch(() => {
            // Already gone with the window.
          });
      }
      if (firstStatusTimer !== null) {
        window.clearTimeout(firstStatusTimer);
      }
    };
  }, [reloaders]);

  /** Auto-start is held by the desktop, not a file the backend saves, so
   *  nothing announces the change: the store rereads settings itself. */
  const setAutostartEnabled = async (enabled: boolean) => {
    try {
      await (enabled ? enableAutostart() : disableAutostart());
      showToastRef.current(`Auto-start ${enabled ? "enabled" : "disabled"}.`, "success");
    } catch (error) {
      showToastRef.current(`Failed to update auto-start: ${error}`, "error");
    }
    reloaders.appData();
  };

  /** The session log, read on demand for the Debug Tools preview. */
  const fetchSessionLogText = async () => {
    try {
      return await readSessionLogText();
    } catch (error) {
      showToastRef.current(`Failed to read session log: ${error}`, "error");
      return "";
    }
  };

  return { status, syncRuntime, settings, systemInfo, setAutostartEnabled, fetchSessionLogText };
}
