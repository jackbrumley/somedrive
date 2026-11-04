import { useCallback, useState } from "preact/hooks";

export type AgentStateRequest = "syncing" | "paused";

/**
 * Tracks one pause or resume request while it is in flight, so its button
 * can say "Pausing…" or "Resuming…" the moment it is clicked. It only
 * describes the click being handled; the sync state itself still comes from
 * the backend once the request finishes.
 */
export function useAgentStateRequest(
  accountId: string,
  setAgentState: (accountId: string, state: AgentStateRequest) => Promise<void>,
) {
  const [pendingState, setPendingState] = useState<AgentStateRequest | null>(null);

  const requestAgentState = useCallback(
    async (state: AgentStateRequest) => {
      if (pendingState !== null) {
        return;
      }
      setPendingState(state);
      try {
        await setAgentState(accountId, state);
      } finally {
        setPendingState(null);
      }
    },
    [accountId, pendingState, setAgentState],
  );

  return { pendingState, requestAgentState };
}

export function agentStateRequestLabel(state: AgentStateRequest): string {
  return state === "paused" ? "Pausing…" : "Resuming…";
}
