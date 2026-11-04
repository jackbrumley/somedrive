import type { LogLevel, SystemInfo } from "../../types/somedrive";
import { SelectField, type SelectFieldOption } from "../ui/SelectField";
import { SettingRow } from "../ui/SettingRow";
import { ToggleSwitch } from "../ui/ToggleSwitch";

interface DebugSettingsCardProps {
  systemInfo: SystemInfo | null;
  logLevel: LogLevel;
  onChangeLogLevel: (level: LogLevel) => Promise<void>;
  rawLoggerMode: boolean;
  onToggleRawLoggerMode: (enabled: boolean) => Promise<void>;
  onCopyLog: () => Promise<void>;
  onOpenLog: () => Promise<void>;
  onOpenLogFolder: () => Promise<void>;
  onCopySystemInfo: () => Promise<void>;
  onGoDebug: () => void;
}

const logLevelOptions: SelectFieldOption[] = [
  { value: "error", label: "Error" },
  { value: "warn", label: "Warn" },
  { value: "info", label: "Info" },
  { value: "debug", label: "Debug" },
  { value: "trace", label: "Trace" },
];

// Settings -> Debug (App Standard section 4): the shared Logs block, then
// SomeDrive's developer options and the Debug Tools page.
export function DebugSettingsCard({
  systemInfo,
  logLevel,
  onChangeLogLevel,
  rawLoggerMode,
  onToggleRawLoggerMode,
  onCopyLog,
  onOpenLog,
  onOpenLogFolder,
  onCopySystemInfo,
  onGoDebug,
}: DebugSettingsCardProps) {
  const systemSummary = systemInfo
    ? `${systemInfo.appName} v${systemInfo.appVersion} (${systemInfo.buildProfile}) · ${systemInfo.distribution} · ${systemInfo.architecture} · ${systemInfo.desktop} (${systemInfo.sessionType})`
    : "System info unavailable.";

  return (
    <article class="card">
      <h3>Debug</h3>
      <div class="settings-list">
        <SettingRow label="Logs" description="For a bug report, attach the copied log and system info.">
          <button type="button" class="ghost" onClick={() => void onCopyLog()}>
            Copy log
          </button>
          <button type="button" class="ghost" onClick={() => void onOpenLog()}>
            Open log
          </button>
          <button type="button" class="ghost" onClick={() => void onOpenLogFolder()}>
            Open folder
          </button>
        </SettingRow>
        <SettingRow label="System info" description={systemSummary}>
          <button type="button" class="ghost" disabled={!systemInfo} onClick={() => void onCopySystemInfo()}>
            Copy
          </button>
        </SettingRow>
        <SettingRow
          label="Log level"
          labelFor="log-level-select"
          description="How much detail the session log records. Info is right for everyday use."
        >
          <SelectField
            id="log-level-select"
            name="log-level"
            value={logLevel}
            options={logLevelOptions}
            onValueChange={(next) => void onChangeLogLevel(next as LogLevel)}
          />
        </SettingRow>
        <ToggleSwitch
          id="raw-logger-mode-toggle"
          label="Raw logger mode"
          description="Write an additional full combined log (every account's detailed sync trace) for deep troubleshooting."
          checked={rawLoggerMode}
          onChange={onToggleRawLoggerMode}
        />
        <SettingRow label="Debug tools" description="Sync diagnostics for each account, the UI Lab and per-account logs.">
          <button type="button" class="ghost" onClick={onGoDebug}>
            Open
          </button>
        </SettingRow>
      </div>
    </article>
  );
}
