import { PageHeader } from "../components/shell/PageHeader";
import { DebugSettingsCard } from "../components/settings/DebugSettingsCard";
import { ExcludeRulesCard } from "../components/settings/ExcludeRulesCard";
import { SettingRow } from "../components/ui/SettingRow";
import { ToggleSwitch } from "../components/ui/ToggleSwitch";
import { SelectField, type SelectFieldOption } from "../components/ui/SelectField";
import type { ConflictPolicy, DownloadConcurrencySetting, LogLevel, SystemInfo } from "../types/somedrive";

const conflictPolicyOptions: SelectFieldOption[] = [
  { value: "ask", label: "Ask me" },
  { value: "keep_newest", label: "Keep the newest" },
];

interface SettingsPageProps {
  autostartEnabled: boolean;
  onToggleAutostart: (enabled: boolean) => Promise<void>;
  closeToTray: boolean;
  onToggleCloseToTray: (enabled: boolean) => Promise<void>;
  rawLoggerMode: boolean;
  onToggleRawLoggerMode: (enabled: boolean) => Promise<void>;
  syncDownloadConcurrency: DownloadConcurrencySetting | null;
  conflictPolicy: ConflictPolicy | null;
  onChangeConflictPolicy: (policy: ConflictPolicy) => Promise<void>;
  excludePatterns: string[] | null;
  onChangeExcludePatterns: (patterns: string[]) => Promise<void>;
  onChangeSyncDownloadConcurrency: (value: number) => Promise<void>;
  appVersion: string;
  checkingUpdates: boolean;
  lastCheckedAt: number | null;
  updateError: string | null;
  onCheckForUpdates: () => void;
  systemInfo: SystemInfo | null;
  logLevel: LogLevel;
  onChangeLogLevel: (level: LogLevel) => Promise<void>;
  onCopyLog: () => Promise<void>;
  onOpenLog: () => Promise<void>;
  onOpenLogFolder: () => Promise<void>;
  onCopySystemInfo: () => Promise<void>;
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
  conflictPolicy,
  onChangeConflictPolicy,
  excludePatterns,
  onChangeExcludePatterns,
  appVersion,
  checkingUpdates,
  lastCheckedAt,
  updateError,
  onCheckForUpdates,
  systemInfo,
  logLevel,
  onChangeLogLevel,
  onCopyLog,
  onOpenLog,
  onOpenLogFolder,
  onCopySystemInfo,
  onGoDebug,
  onBack,
}: SettingsPageProps) {
  const lastCheckedLabel = lastCheckedAt === null ? "Not yet" : new Date(lastCheckedAt).toLocaleString();

  return (
    <section class="page">
      <PageHeader title="Settings" back={{ href: "#/accounts", label: "Back to accounts", onBack }} />
      <div class="page-scroll">
        <article class="card">
          <h3>General</h3>
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
          <div class="settings-list">
            <SettingRow
              label={`SomeDrive v${appVersion}`}
              description={
                <>
                  Last checked: {lastCheckedLabel}. Updates install in place and sync resumes after the restart.
                  {updateError && <span class="setting-row-error">Last check failed: {updateError}</span>}
                </>
              }
            >
              <button type="button" class="ghost" onClick={onCheckForUpdates} disabled={checkingUpdates}>
                {checkingUpdates ? "Checking…" : "Check for updates"}
              </button>
            </SettingRow>
          </div>
        </article>

        <article class="card">
          <h3>Sync</h3>
          <div class="settings-list">
            <SettingRow
              label="When a file changes on both sides"
              labelFor="conflict-policy-select"
              description="Ask me holds the file in Sync problems until you choose. Keep the newest replaces the older version, which stays in OneDrive's version history or your Trash."
            >
              {conflictPolicy === null ? (
                <span class="setting-row-description">Unavailable</span>
              ) : (
                <SelectField
                  id="conflict-policy-select"
                  name="conflict-policy"
                  value={conflictPolicy}
                  options={conflictPolicyOptions}
                  onValueChange={(next) => void onChangeConflictPolicy(next as ConflictPolicy)}
                />
              )}
            </SettingRow>
            {syncDownloadConcurrency === null ? (
              <SettingRow label="Simultaneous downloads" description="This setting could not be loaded." />
            ) : (
              <DownloadConcurrencySlider
                setting={syncDownloadConcurrency}
                onChange={onChangeSyncDownloadConcurrency}
              />
            )}
          </div>
        </article>

        <ExcludeRulesCard patterns={excludePatterns} onChange={onChangeExcludePatterns} />

        <DebugSettingsCard
          systemInfo={systemInfo}
          logLevel={logLevel}
          onChangeLogLevel={onChangeLogLevel}
          rawLoggerMode={rawLoggerMode}
          onToggleRawLoggerMode={onToggleRawLoggerMode}
          onCopyLog={onCopyLog}
          onOpenLog={onOpenLog}
          onOpenLogFolder={onOpenLogFolder}
          onCopySystemInfo={onCopySystemInfo}
          onGoDebug={onGoDebug}
        />
      </div>
    </section>
  );
}

interface DownloadConcurrencySliderProps {
  setting: DownloadConcurrencySetting;
  onChange: (value: number) => Promise<void>;
}

function DownloadConcurrencySlider({ setting, onChange }: DownloadConcurrencySliderProps) {
  const progress = (setting.value - setting.min) / (setting.max - setting.min);
  return (
    <SettingRow
      label="Simultaneous downloads"
      labelFor="download-concurrency-slider"
      description={`More is faster, but Microsoft may slow SomeDrive down. ${setting.min} to ${setting.max}; applies from the next sync cycle.`}
    >
      <div class="settings-range-control">
        <input
          id="download-concurrency-slider"
          type="range"
          min={setting.min}
          max={setting.max}
          step={1}
          value={setting.value}
          style={{ "--settings-range-progress": String(progress) } as Record<string, string>}
          onChange={(event) => {
            const input = event.currentTarget as HTMLInputElement;
            void onChange(Number.parseInt(input.value, 10));
          }}
        />
        <output class="settings-range-value" htmlFor="download-concurrency-slider">
          {setting.value}
        </output>
      </div>
    </SettingRow>
  );
}
