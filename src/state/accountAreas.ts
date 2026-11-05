import { createContext } from "preact";
import { useContext } from "preact/hooks";
import type { CleanupView, SyncProblemsView } from "../types/somedrive";
import { readCleanup, readLargeDeletePreview, readSyncProblems } from "./backendReads";
import { useRevisionedRead, type AreaState } from "./useRevisionedRead";

/**
 * Where an account page gets the lists that are not part of the live status.
 * Each is reread when the account's change counter for it moves
 * (`SyncRuntimeAccountStatus.dataRevisions`). The UI Lab supplies sample
 * data through the same context instead of the backend.
 */
export interface AccountAreaSource {
  useSyncProblems: (accountId: string, revision: number | undefined) => AreaState<SyncProblemsView>;
  /** Read only while the Clean up tab is open: the analysis covers every file. */
  useCleanup: (accountId: string, revision: number | undefined) => AreaState<CleanupView>;
  /** Read only while deletes are held for review. */
  useLargeDeletePreview: (accountId: string, revision: number | undefined, held: boolean) => AreaState<string[]>;
}

export const backendAccountAreas: AccountAreaSource = {
  useSyncProblems: (accountId, revision) => useRevisionedRead(accountId, revision, readSyncProblems),
  useCleanup: (accountId, revision) => useRevisionedRead(accountId, revision, readCleanup),
  useLargeDeletePreview: (accountId, revision, held) =>
    useRevisionedRead(accountId, revision, readLargeDeletePreview, held),
};

export const AccountAreasContext = createContext<AccountAreaSource>(backendAccountAreas);

export function useAccountAreas(): AccountAreaSource {
  return useContext(AccountAreasContext);
}
