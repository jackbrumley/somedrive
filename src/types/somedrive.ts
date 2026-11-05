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
  lastSyncAt?: string | null;
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

/** Mirrors `sync_engine::NotSyncedItem`: an item skipped by design. */
export interface NotSyncedItem {
  path: string;
  reasonCode: "onenote_notebook" | "excluded_by_rule" | "malware_detected" | "shared_with_you" | string;
  reason: string;
  isFolder: boolean;
  contained: number;
}

/** Mirrors `sync_engine::SyncProblemsView`. */
export interface SyncProblemsView {
  problems: SyncProblem[];
  notSynced: NotSyncedItem[];
}

/** Mirrors `sync_engine::KeptLocation`: where identical copies stay. */
export interface KeptLocation {
  path: string;
  fileCount: number;
}

/** Mirrors `sync_engine::DuplicatePair`: a file to delete and the identical
 *  copy that stays. */
export interface DuplicatePair {
  path: string;
  keptPath: string;
  size: number;
}

/** Mirrors `sync_engine::DuplicateSuggestion`. `folder`: the whole folder;
 *  `partial`: the identical part of a folder; `files`: identical files
 *  directly in a folder. */
export interface DuplicateSuggestion {
  id: string;
  kind: "folder" | "partial" | "files";
  path: string;
  deletePaths: string[];
  fileCount: number;
  bytes: number;
  folderFileCount: number;
  keptIn: KeptLocation[];
  keptInMore: number;
  pairs: DuplicatePair[];
}

/** Mirrors `sync_engine::LargeFile`. */
export interface LargeFile {
  path: string;
  size: number;
  modified: number;
  copies: string[];
}

/** Mirrors `sync_engine::CleanupView`. */
export interface CleanupView {
  duplicates: DuplicateSuggestion[];
  largeFiles: LargeFile[];
  duplicateBytes: number;
  hidden: string[];
}

/** Mirrors `sync_engine::CleanupDeleteResult`. */
export interface CleanupDeleteResult {
  fileCount: number;
  bytes: number;
}

/** The Clean up tab's backend calls. The deletes reject with the reason
 *  when nothing was deleted. */
export interface CleanupActions {
  list: (accountId: string) => Promise<CleanupView>;
  deleteDuplicates: (accountId: string, suggestionId: string) => Promise<CleanupDeleteResult>;
  deleteLargeFile: (accountId: string, path: string) => Promise<CleanupDeleteResult>;
  hide: (accountId: string, path: string) => Promise<void>;
  unhide: (accountId: string, path: string) => Promise<void>;
  /** Copies text (the file name to type before deleting) to the clipboard; resolves true when it worked. */
  copyText: (text: string) => Promise<boolean>;
}

/** Mirrors `app_settings::ConflictPolicy`. */
export type ConflictPolicy = "ask" | "keep_newest";
