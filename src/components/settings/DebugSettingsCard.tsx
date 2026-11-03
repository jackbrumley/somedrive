import type { LogLevel, SystemInfo } from "../../types/somedrive";
import { SelectField, type SelectFieldOption } from "../ui/SelectField";
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
  return (
    <article class="card">
      <h3>Debug</h3>
      <h4>Logs</h4>
      <p>For a bug report, attach the copied log and system info.</p>
      <div class="button-row">
        <button onClick={() => void onCopyLog()}>Copy log</button>
        <button class="ghost" onClick={() => void onOpenLog()}>
          Open log
        </button>
        <button class="ghost" onClick={() => void onOpenLogFolder()}>
          Open log folder
        </button>
      </div>

      <h4>System info</h4>
      {systemInfo ? (
        <dl class="system-info">
          <dt>Version</dt>
          <dd>
            {systemInfo.appName} v{systemInfo.appVersion} ({systemInfo.buildProfile})
          </dd>
          <dt>OS</dt>
          <dd>
            {systemInfo.distribution} ({systemInfo.os})
          </dd>
          <dt>Architecture</dt>
          <dd>{systemInfo.architecture}</dd>
          <dt>Desktop</dt>
          <dd>
            {systemInfo.desktop} ({systemInfo.sessionType})
          </dd>
        </dl>
      ) : (
        <p>System info unavailable.</p>
      )}
      <div class="button-row">
        <button class="ghost" onClick={() => void onCopySystemInfo()}>
          Copy system info
        </button>
      </div>

      <label class="field-label" for="log-level-select">
        Log level
      </label>
      <SelectField
        id="log-level-select"
        name="log-level"
        value={logLevel}
        options={logLevelOptions}
        onValueChange={(next) => void onChangeLogLevel(next as LogLevel)}
      />

      <div class="settings-list">
        <ToggleSwitch
          id="raw-logger-mode-toggle"
          label="Raw logger mode"
          description="Write an additional full combined log (every account's detailed sync trace) for deep troubleshooting."
          checked={rawLoggerMode}
          onChange={onToggleRawLoggerMode}
        />
      </div>
      <div class="button-row">
        <button class="ghost" onClick={onGoDebug}>
          Open Debug Tools
        </button>
      </div>
    </article>
  );
}
