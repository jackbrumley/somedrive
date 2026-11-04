export type AccountKind = "personal" | "business";

export type SyncAgentState = "idle" | "syncing" | "paused" | "error";

export interface AccountProfile {
  id: string;
  displayName: string;
  email: string;
  slug: string;
  kind: AccountKind;
  syncRoot: string;
  authConfigured: boolean;
  agentState: SyncAgentState;
  lastSyncAt: string | null;
}

export interface ActivityEvent {
  id: string;
  profileId: string;
  profileName: string;
  kind: "info" | "success" | "warning" | "error";
  message: string;
  timestamp: string;
}

export interface DeviceAuthSession {
  profileId: string;
  userCode: string;
  verificationUri: string;
  verificationUriComplete: string | null;
  expiresIn: number;
  interval: number;
  message: string;
}

export interface DeviceAuthPollResult {
  status: "pending" | "authorized" | "error";
  detail: string;
}

export interface CreateAccountProfileInput {
  displayName: string;
  kind: AccountKind;
}

export interface RenameAccountProfileInput {
  id: string;
  displayName: string;
}

export interface SetAccountAgentStateInput {
  id: string;
  agentState: SyncAgentState;
}

export interface SetAccountSyncRootInput {
  id: string;
  syncRoot: string;
}

export interface AppStatusSnapshot {
  appVersion: string;
  platform: string;
  syncEngineReady: boolean;
  authConfigured: boolean;
  activeAccount: string | null;
  lastSyncAt: string | null;
  health: "ok" | "degraded" | "error";
  accounts: AccountProfile[];
}

export interface UpdateCheckResult {
  currentVersion: string;
  latestVersion: string;
  updateAvailable: boolean;
  releaseUrl: string;
  notesUrl: string | null;
}

// "finishing_sync": the backend is draining sync before the installer starts.
// "starting": the installer is running and SomeDrive is about to exit.
export type UpdateInstallStage = "idle" | "finishing_sync" | "starting";

export interface SyncRuntimeTransfer {
  id: string;
  direction: string;
  path: string;
  state?: string;
  bytesDone: number;
  bytesTotal: number | null;
  startedAt: string;
  updatedAt: string;
}

export interface SyncRuntimeRecentItem {
  id: string;
  direction: string;
  path: string;
  bytesTotal: number | null;
  finishedAt: string;
  status: string;
  error: string | null;
}

export interface SyncRuntimeCurrentActivity {
  stage: string;
  progressMode: "determinate" | "indeterminate" | "hidden" | string;
  current?: number | null;
  total?: number | null;
  unit?: string | null;
  detail?: string | null;
  cycleId?: string | null;
  updatedAt?: string;
}

export interface SyncRuntimeConsistency {
  ok: boolean;
  violations: string[];
}

export interface SyncRuntimeAccountStatus {
  profileId: string;
  engineState?: "running" | "paused" | "blocked" | string;
  phase: string;
  phaseMessage: string;
  issueCode: string | null;
  issueMessage: string | null;
  issueActions: string[];
  issuePath: string | null;
  issueSecondaryPath: string | null;
  issueSeverity?: "none" | "warning" | "blocking" | string;
  authReady?: boolean;
  canSync?: boolean;
  inProgress: SyncRuntimeTransfer[];
  recentCompleted: SyncRuntimeRecentItem[];
  recentRetryWaiting: SyncRuntimeRecentItem[];
  recentFailed: SyncRuntimeRecentItem[];
  plannerCloudDiscoveredTotal?: number;
  plannerLocalDiscoveredTotal?: number;
  plannerNoneTotal?: number;
  plannerNeedDownloadTotal?: number;
  plannerNeedUploadTotal?: number;
  plannerNeedDeleteRemoteTotal?: number;
  plannerNeedDeleteLocalTotal?: number;
  plannerConflictTotal?: number;
  remoteDiscoveredTotal?: number;
  remoteDownloadPlannedTotal?: number;
  remoteDownloadCompletedTotal?: number;
  remoteDownloadFailedTotal?: number;
  remoteDownloadInFlight?: number;
  remoteDownloadRetryWaiting?: number;
  remoteDownloadPlannedBytesTotal?: number;
  remoteDownloadCompletedBytesTotal?: number;
  remoteDownloadRemainingBytesTotal?: number;
  remoteDownloadInFlightBytesDone?: number;
  remoteDownloadThrottleTotal?: number;
  remoteDownloadThrottleLastMinute?: number;
  uploadPlannedTotal?: number;
  uploadCompletedTotal?: number;
  uploadFailedTotal?: number;
  uploadInFlight?: number;
  uploadRetryWaiting?: number;
  uploadPlannedBytesTotal?: number;
  uploadCompletedBytesTotal?: number;
  uploadRemainingBytesTotal?: number;
  uploadInFlightBytesDone?: number;
  uploadThrottleTotal?: number;
  uploadThrottleLastMinute?: number;
  remoteScanComplete?: boolean;
  localScanScannedCount?: number;
  localScanEstimatedTotal?: number | null;
  localScanCurrentPath?: string | null;
  currentActivity?: SyncRuntimeCurrentActivity;
  consistency?: SyncRuntimeConsistency;
  updatedAt: string;
}

export interface SyncRuntimeSnapshot {
  generatedAt: string;
  revision: number;
  accounts: SyncRuntimeAccountStatus[];
}

export interface SyncStatusEvent {
  profileId: string;
  statusSeq: number;
  generatedAt: string;
  kind: "upsert" | "removed" | string;
  status: SyncRuntimeAccountStatus | null;
}

export type ToastType = "success" | "error" | "info";

// Mirrors `app::system_info::SystemInfo`.
export interface SystemInfo {
  appName: string;
  appVersion: string;
  buildProfile: string;
  executable: string;
  os: string;
  distribution: string;
  architecture: string;
  desktop: string;
  sessionType: string;
}

export type LogLevel = "error" | "warn" | "info" | "debug" | "trace";

export interface ToastMessage {
  id: number;
  message: string;
  type: ToastType;
  durationMs: number;
}

/** Download concurrency with the range the backend accepts (it owns the limits). */
export interface DownloadConcurrencySetting {
  value: number;
  min: number;
  max: number;
}

/** How the user resolves a file that differs on this device and in OneDrive. */
export type ConflictResolution = "keep_local" | "keep_cloud" | "keep_both";

/** Mirrors `sync_engine::SyncProblem`: one file that needs the user. */
export interface SyncProblem {
  path: string;
  code: string;
  detail: string;
  isConflict: boolean;
  resolution: ConflictResolution | null;
  localSize: number | null;
  localModifiedAt: string | null;
  cloudSize: number | null;
  cloudModifiedAt: string | null;
  retryId: string | null;
}

/** Mirrors `app_settings::ConflictPolicy`. */
export type ConflictPolicy = "ask" | "keep_newest";
