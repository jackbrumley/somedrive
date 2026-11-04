import { useCallback, useEffect, useRef, useState } from "preact/hooks";
import type { ConflictResolution, NotSyncedItem, SyncProblem, SyncProblemsView } from "../types/somedrive";

/** How often (at most) the list is re-read while sync status keeps changing. */
const REFRESH_INTERVAL_MS = 3000;

interface SyncProblemActions {
  fetchProblems: (accountId: string) => Promise<SyncProblemsView>;
  resolveConflicts: (accountId: string, paths: string[], resolution: ConflictResolution) => Promise<void>;
}

interface SyncProblemsState {
  problems: SyncProblem[];
  notSynced: NotSyncedItem[];
  resolve: (paths: string[], resolution: ConflictResolution) => Promise<void>;
}

/**
 * The account's sync problems (held conflicts, items that cannot sync,
 * failed downloads), re-read as its sync status changes.
 */
export function useSyncProblems(
  accountId: string,
  statusUpdatedAt: string | undefined,
  actions: SyncProblemActions
): SyncProblemsState {
  const [problems, setProblems] = useState<SyncProblem[]>([]);
  const [notSynced, setNotSynced] = useState<NotSyncedItem[]>([]);
  // The actions are recreated on every render upstream; reading them through
  // a ref keeps the refresh keyed on the account and status alone.
  const actionsRef = useRef(actions);
  actionsRef.current = actions;

  const refresh = useCallback(async () => {
    const view = await actionsRef.current.fetchProblems(accountId);
    setProblems(view.problems);
    setNotSynced(view.notSynced);
  }, [accountId]);

  const refreshBucket = statusUpdatedAt ? Math.floor(Date.parse(statusUpdatedAt) / REFRESH_INTERVAL_MS) : 0;
  useEffect(() => {
    void refresh();
  }, [refresh, refreshBucket]);

  const resolve = useCallback(
    async (paths: string[], resolution: ConflictResolution) => {
      await actionsRef.current.resolveConflicts(accountId, paths, resolution);
      await refresh();
    },
    [accountId, refresh]
  );

  return { problems, notSynced, resolve };
}
