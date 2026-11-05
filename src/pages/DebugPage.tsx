import { IconFileText } from "@tabler/icons-preact";
import { useState } from "preact/hooks";
import { PageHeader } from "../components/shell/PageHeader";
import { SettingRow } from "../components/ui/SettingRow";
import type { AppStatusSnapshot } from "../types/somedrive";

const SESSION_LOG_PREVIEW_LINES = 14;

interface DebugPageProps {
  status: AppStatusSnapshot;
  onBack: () => void;
  onNavigateUiLab: () => void;
  onFetchSessionLogText: () => Promise<string>;
  onOpenProfileLog: (profileId: string) => Promise<void>;
}

/** Developer tools. Each account's sync details are on its own sync page. */
export function DebugPage({
  status,
  onBack,
  onNavigateUiLab,
  onFetchSessionLogText,
  onOpenProfileLog,
}: DebugPageProps) {
  const [logPreview, setLogPreview] = useState<string | null>(null);
  const [loadingLog, setLoadingLog] = useState(false);

  const loadLog = async () => {
    setLoadingLog(true);
    const text = await onFetchSessionLogText();
    setLogPreview(text.split("\n").slice(-SESSION_LOG_PREVIEW_LINES).join("\n"));
    setLoadingLog(false);
  };

  return (
    <section class="page">
      <PageHeader title="Debug Tools" back={{ href: "#/settings", label: "Back to settings", onBack }} />
      <div class="page-scroll">
        <article class="card">
          <h3>Runtime</h3>
          <dl class="facts-grid">
            <div>
              <dt>Version</dt>
              <dd>v{status.appVersion}</dd>
            </div>
            <div>
              <dt>Platform</dt>
              <dd>{status.platform}</dd>
            </div>
            <div>
              <dt>Health</dt>
              <dd>{status.health}</dd>
            </div>
            <div>
              <dt>Accounts</dt>
              <dd>{status.accounts.length}</dd>
            </div>
          </dl>
          <div class="settings-list">
            <SettingRow label="UI Lab" description="Preview account states and screen variants while tuning the design.">
              <button type="button" class="ghost" onClick={onNavigateUiLab}>
                Open
              </button>
            </SettingRow>
          </div>
        </article>

        <article class="card">
          <h3>Logs</h3>
          <div class="settings-list">
            <SettingRow label="Session log" description="The last lines of this session's log. Copy and open it from Settings, under Debug.">
              <button type="button" class="ghost" onClick={() => void loadLog()} disabled={loadingLog}>
                {loadingLog ? "Loading…" : logPreview === null ? "Show" : "Reload"}
              </button>
            </SettingRow>
            {logPreview !== null && <pre class="log-preview">{logPreview}</pre>}
            {status.accounts.map((account) => (
              <SettingRow
                key={account.id}
                label={`${account.displayName || account.id} sync log`}
                description="This account's detailed file and delta activity."
              >
                <button type="button" class="ghost" onClick={() => void onOpenProfileLog(account.id)}>
                  <IconFileText size={15} />
                  Open
                </button>
              </SettingRow>
            ))}
          </div>
        </article>
      </div>
    </section>
  );
}
