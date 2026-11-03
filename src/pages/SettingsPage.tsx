import { IconChevronLeft } from "@tabler/icons-preact";
import { ToggleSwitch } from "../components/ui/ToggleSwitch";

interface SettingsPageProps {
  autostartEnabled: boolean;
  onToggleAutostart: (enabled: boolean) => Promise<void>;
  closeToTray: boolean;
  onToggleCloseToTray: (enabled: boolean) => Promise<void>;
  rawLoggerMode: boolean;
  onToggleRawLoggerMode: (enabled: boolean) => Promise<void>;
  syncDownloadConcurrency: number;
  onChangeSyncDownloadConcurrency: (value: number) => Promise<void>;
  appVersion: string;
  checkingUpdates: boolean;
  lastCheckedAt: number | null;
  updateError: string | null;
  onCheckForUpdates: () => void;
  onGoDebug: () => void;
  onBack: () => void;
}

export function SettingsPage({
  autostartEnabled,
  onToggleAutostart,
  closeToTray,
  onToggleCloseToTray,
  rawLoggerMode,
  onToggleRawLoggerMode,
  syncDownloadConcurrency,
  onChangeSyncDownloadConcurrency,
  appVersion,
  checkingUpdates,
  lastCheckedAt,
  updateError,
  onCheckForUpdates,
  onGoDebug,
  onBack,
}: SettingsPageProps) {
  const downloadConcurrencyMin = 8;
  const downloadConcurrencyMax = 256;
  const concurrencyProgress = Math.min(
    1,
    Math.max(
      0,
      (syncDownloadConcurrency - downloadConcurrencyMin) /
        (downloadConcurrencyMax - downloadConcurrencyMin)
    )
  );
  const lastCheckedLabel = lastCheckedAt === null ? "Not yet" : new Date(lastCheckedAt).toLocaleString();

  return (
    <section class="page">
      <div class="page-chrome">
        <div class="page-header">
          <a
            class="page-header-back-link"
            href="#/accounts"
            onClick={(event) => {
              event.preventDefault();
              onBack();
            }}
            aria-label="Back to accounts"
            title="Back to accounts"
          >
            <IconChevronLeft size={36} stroke={2.2} />
          </a>
          <h2>Settings</h2>
        </div>
      </div>
      <div class="page-scroll">
        <article class="card">
          <h3>General</h3>
          <p>Set how SomeDrive starts up and closes on this device.</p>
          <div class="settings-list">
            <ToggleSwitch
              id="autostart-toggle"
              label="Start SomeDrive when I log in"
              description="Starts in the system tray and resumes syncing in the background."
              checked={autostartEnabled}
              onChange={onToggleAutostart}
            />
            <ToggleSwitch
              id="close-to-tray-toggle"
              label="Close to tray"
              description="Closing the window keeps SomeDrive syncing in the background. Turn off to quit when the window closes."
              checked={closeToTray}
              onChange={onToggleCloseToTray}
            />
          </div>
        </article>

        <article class="card">
          <h3>Updates</h3>
          <p>
            You are running SomeDrive v{appVersion}. Updates install in place and sync resumes after the restart.
          </p>
          <p class="settings-update-meta">Last checked: {lastCheckedLabel}</p>
          {updateError && <p class="settings-update-meta error">Last check failed: {updateError}</p>}
          <div class="button-row">
            <button onClick={onCheckForUpdates} disabled={checkingUpdates}>
              {checkingUpdates ? "Checking…" : "Check for Updates"}
            </button>
          </div>
        </article>

        <article class="card">
          <h3>Sync Performance</h3>
          <p>Set the number of simultaneous cloud downloads. Higher values may increase speed but can trigger retries.</p>
          <div class="settings-list">
            <label class="settings-range-field" htmlFor="download-concurrency-slider">
              <span class="settings-range-label">Simultaneous downloads</span>
              <div class="settings-range-control">
                <input
                  id="download-concurrency-slider"
                  type="range"
                  min={downloadConcurrencyMin}
                  max={downloadConcurrencyMax}
                  step={1}
                  value={syncDownloadConcurrency}
                  style={{
                    "--settings-range-progress": String(concurrencyProgress),
                  } as Record<string, string>}
                  onChange={(event) => {
                    const input = event.currentTarget as HTMLInputElement;
                    void onChangeSyncDownloadConcurrency(Number.parseInt(input.value, 10));
                  }}
                />
                <output class="settings-range-value" htmlFor="download-concurrency-slider">
                  {syncDownloadConcurrency}
                </output>
              </div>
              <span class="settings-range-description">Range: 8 to 256. Changes apply to subsequent sync cycles.</span>
            </label>
          </div>
        </article>

        <article class="card">
          <h3>Developer</h3>
          <p>Debug and diagnostics tools are grouped separately from regular settings.</p>
          <div class="settings-list">
            <ToggleSwitch
              id="raw-logger-mode-toggle"
              label="Raw logger mode"
              description="Write an additional full combined session log for deep troubleshooting."
              checked={rawLoggerMode}
              onChange={onToggleRawLoggerMode}
            />
          </div>
          <div class="button-row">
            <button onClick={onGoDebug}>Open Debug Tools</button>
          </div>
        </article>
        </div>
    </section>
  );
}
