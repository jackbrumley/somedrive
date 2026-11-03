import { useState } from "preact/hooks";
import { IconChevronLeft } from "@tabler/icons-preact";
import type { AppStatusSnapshot } from "../types/somedrive";

interface DebugPageProps {
  status: AppStatusSnapshot;
  onBack: () => void;
  onNavigateUiLab: () => void;
  onRefreshStatus: () => Promise<void>;
  onFetchSessionLogText: () => Promise<string>;
  onOpenProfileLog: (profileId: string) => Promise<void>;
}

export function DebugPage({
  status,
  onBack,
  onNavigateUiLab,
  onRefreshStatus,
  onFetchSessionLogText,
  onOpenProfileLog,
}: DebugPageProps) {
  const [logPreview, setLogPreview] = useState("");
  const [loadingLog, setLoadingLog] = useState(false);

  const loadLog = async () => {
    setLoadingLog(true);
    const text = await onFetchSessionLogText();
    const lines = text.split("\n").slice(-14).join("\n");
    setLogPreview(lines);
    setLoadingLog(false);
  };

  return (
    <section class="page">
      <div class="page-chrome">
        <div class="page-header">
          <a
            class="page-header-back-link"
            href="#/settings"
            onClick={(event) => {
              event.preventDefault();
              onBack();
            }}
            aria-label="Back to settings"
            title="Back to settings"
          >
            <IconChevronLeft size={36} stroke={2.2} />
          </a>
          <h2>Debug Tools</h2>
        </div>
      </div>
      <div class="page-scroll">
        <article class="card">
          <h3>Development Tools</h3>
          <p>
            Use the UI Lab to preview account states and screen variants while tuning look and feel. Copy log, Open
            log and System info are in Settings, under Debug.
          </p>
          <div class="button-row">
            <button onClick={onNavigateUiLab}>Open UI Lab</button>
            <button onClick={onRefreshStatus}>Refresh Runtime State</button>
            <button onClick={loadLog} disabled={loadingLog}>
              {loadingLog ? "Loading..." : "Load Session Log Preview"}
            </button>
          </div>
        </article>

        <article class="card">
          <h3>Runtime Snapshot</h3>
          <p>Platform: {status.platform}</p>
          <p>Version: v{status.appVersion}</p>
          <p>Health: {status.health}</p>
          <p>Profile Count: {status.accounts.length}</p>
        </article>

        <article class="card">
          <h3>Recent Session Log</h3>
          {logPreview ? <pre class="log-preview">{logPreview}</pre> : <p>Log preview not loaded yet.</p>}
        </article>

        <article class="card">
          <h3>Profile Logs</h3>
          <p>Open per-profile sync logs for detailed file and delta activity.</p>
          <div class="button-row">
            {status.accounts.length === 0 ? (
              <span>No profiles available.</span>
            ) : (
              status.accounts.map((account) => (
                <button key={account.id} onClick={() => void onOpenProfileLog(account.id)}>
                  Open {account.displayName || account.id} Log
                </button>
              ))
            )}
          </div>
        </article>
      </div>
    </section>
  );
}
