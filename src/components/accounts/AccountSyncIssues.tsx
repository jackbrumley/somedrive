import { IconLogin2 } from "@tabler/icons-preact";
import { FileTypeIcon } from "./FileTypeIcon";
import { splitSyncPath } from "./syncFormatting";

const DELETE_PREVIEW_LIMIT = 40;

interface AccountSyncIssuesProps {
  issueMessage: string | null;
  issueKind: "auth_required" | "sync_error" | null;
  issueActions: string[];
  issuePath: string | null;
  issueSecondaryPath: string | null;
  largeDeletePreviewPaths: string[];
  onOpenItemFolder: (relativePath: string) => Promise<void>;
  onOpenSyncRootFolder: () => Promise<void>;
  onReauthenticate: () => Promise<unknown>;
  onRetrySync: () => Promise<void>;
  onConfirmLargeDelete: () => Promise<void>;
  onKeepCloudFiles: () => Promise<void>;
  onExportLargeDeletePreview: () => Promise<void>;
}

function SignInPrompt({ message, onReauthenticate }: { message: string | null; onReauthenticate: () => Promise<unknown> }) {
  return (
    <section class="sync-issue-card sync-issue-card-auth">
      <div>
        <h4>Sign in to continue</h4>
        <p>{message ?? "Your Microsoft sign-in has expired. Sign in again to resume syncing."}</p>
        <p class="sync-issue-hint">Files already on this device stay where they are.</p>
      </div>
      <button type="button" onClick={() => void onReauthenticate()}>
        <IconLogin2 size={15} />
        Sign in to Microsoft
      </button>
    </section>
  );
}

export function AccountSyncIssues({
  issueMessage,
  issueKind,
  issueActions,
  issuePath,
  issueSecondaryPath,
  largeDeletePreviewPaths,
  onOpenItemFolder,
  onOpenSyncRootFolder,
  onReauthenticate,
  onRetrySync,
  onConfirmLargeDelete,
  onKeepCloudFiles,
  onExportLargeDeletePreview,
}: AccountSyncIssuesProps) {
  const conflictTargetPath = issueSecondaryPath ?? issuePath;
  const hasSyncErrorCard = issueKind === "sync_error" && (Boolean(issueMessage) || issueActions.length > 0);

  return (
    <>
      {issueKind === "auth_required" && <SignInPrompt message={issueMessage} onReauthenticate={onReauthenticate} />}

      {hasSyncErrorCard && (
        <section class="sync-issue-card">
          <h4>What happened</h4>
          {issueMessage && <p class="sync-issue-message">{issueMessage}</p>}
          <div class="sync-issue-actions">
            {issueActions.includes("retry_sync") && (
              <button type="button" onClick={() => void onRetrySync()}>
                Retry sync
              </button>
            )}
            {issueActions.includes("open_sync_root") && (
              <button type="button" class="ghost" onClick={() => void onOpenSyncRootFolder()}>
                Open sync folder
              </button>
            )}
            {issueActions.includes("open_conflict") && conflictTargetPath && (
              <button type="button" class="ghost" onClick={() => void onOpenItemFolder(conflictTargetPath)}>
                Open conflict
              </button>
            )}
            {issueActions.includes("confirm_large_delete") && (
              <button type="button" class="danger" onClick={() => void onConfirmLargeDelete()}>
                Delete from cloud
              </button>
            )}
            {issueActions.includes("keep_cloud_files") && (
              <button type="button" class="ghost" onClick={() => void onKeepCloudFiles()}>
                Keep cloud files
              </button>
            )}
          </div>
        </section>
      )}

      {largeDeletePreviewPaths.length > 0 && (
        <section class="sync-issue-card">
          <div class="sync-section-header">
            <h4>Review deletions ({largeDeletePreviewPaths.length.toLocaleString()})</h4>
            <button type="button" class="ghost" onClick={() => void onExportLargeDeletePreview()}>
              Export full list
            </button>
          </div>
          <ul class="sync-file-list">
            {largeDeletePreviewPaths.slice(0, DELETE_PREVIEW_LIMIT).map((path) => {
              const { name, folder } = splitSyncPath(path);
              return (
                <li key={path} class="sync-file-row">
                  <button type="button" class="sync-file-open" onClick={() => void onOpenItemFolder(path)}>
                    <span class="sync-file-icon">
                      <FileTypeIcon path={path} size={20} />
                    </span>
                    <span class="sync-file-text">
                      <span class="sync-file-name">{name}</span>
                      {folder && <span class="sync-file-folder">{folder}</span>}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
          {largeDeletePreviewPaths.length > DELETE_PREVIEW_LIMIT && (
            <p class="sync-issue-hint">Showing the first {DELETE_PREVIEW_LIMIT}. Export the full list to see them all.</p>
          )}
        </section>
      )}

    </>
  );
}
