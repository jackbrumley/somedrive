import { useEffect, useMemo, useState } from "preact/hooks";
import { PageHeader } from "../components/shell/PageHeader";
import { AccountCard } from "../components/accounts/AccountCard";
import { AccountDetailPage } from "./AccountDetailPage";
import type {
  AccountProfile,
  SyncAgentState,
  SyncRuntimeAccountStatus,
} from "../types/somedrive";

interface UiLabPageProps {
  onBack: () => void;
}

type UiLabAccountView = "sync" | "settings";

function parseUiLabRoute(hash: string): { accountId: string; view: UiLabAccountView } | null {
  const segments = hash.replace(/^#\/?/, "").split("/").filter(Boolean);
  if (segments[0] !== "ui-lab" || segments[1] !== "accounts" || !segments[2]) {
    return null;
  }
  return {
    accountId: decodeURIComponent(segments[2]),
    view: segments[3] === "settings" ? "settings" : "sync",
  };
}

function buildUiLabRoute(accountId: string | null, view: UiLabAccountView): string {
  if (!accountId) {
    return "#/ui-lab";
  }
  const base = `#/ui-lab/accounts/${encodeURIComponent(accountId)}`;
  return view === "settings" ? `${base}/settings` : base;
}

const previewAccounts: AccountProfile[] = [
  {
    id: "lab-1",
    displayName: "Personal",
    email: "personal@example.com",
    slug: "personal",
    kind: "personal",
    syncRoot: "/home/user/SomeDrive/personal",
    authConfigured: true,
    agentState: "syncing",
    lastSyncAt: "2026-05-07T10:24:00.000Z",
  },
  {
    id: "lab-2",
    displayName: "Work",
    email: "work@example.com",
    slug: "work",
    kind: "business",
    syncRoot: "/home/user/SomeDrive/work",
    authConfigured: true,
    agentState: "paused",
    lastSyncAt: null,
  },
  {
    id: "lab-3",
    displayName: "Design Team",
    email: "design@example.com",
    slug: "design-team",
    kind: "business",
    syncRoot: "/home/user/SomeDrive/design",
    authConfigured: true,
    agentState: "syncing",
    lastSyncAt: "2026-05-07T10:22:00.000Z",
  },
  {
    id: "lab-4",
    displayName: "Archive",
    email: "archive@example.com",
    slug: "archive",
    kind: "personal",
    syncRoot: "/home/user/SomeDrive/archive",
    authConfigured: true,
    agentState: "paused",
    lastSyncAt: "2026-05-06T09:00:00.000Z",
  },
  {
    id: "lab-5",
    displayName: "Personal 2",
    email: "",
    slug: "personal-2",
    kind: "personal",
    syncRoot: "/home/user/SomeDrive/personal-2",
    authConfigured: false,
    agentState: "error",
    lastSyncAt: null,
  },
];

export function UiLabPage({ onBack }: UiLabPageProps) {
  const initialLabRoute = parseUiLabRoute(window.location.hash);
  const [selectedLabAccountId, setSelectedLabAccountId] = useState<string | null>(initialLabRoute?.accountId ?? null);
  const [selectedLabAccountView, setSelectedLabAccountView] = useState<UiLabAccountView>(initialLabRoute?.view ?? "sync");
  const [labAgentStateById, setLabAgentStateById] = useState<Record<string, SyncAgentState>>({});

  const navigateUiLab = (accountId: string | null, view: UiLabAccountView = "sync") => {
    const nextHash = buildUiLabRoute(accountId, view);
    if (window.location.hash !== nextHash) {
      window.location.hash = nextHash;
    }
    setSelectedLabAccountId(accountId);
    setSelectedLabAccountView(view);
  };

  const accounts = useMemo(() => {
    return previewAccounts.map((account) => ({
      ...account,
      agentState: labAgentStateById[account.id] ?? account.agentState,
    }));
  }, [labAgentStateById]);

  const selectedLabAccount = selectedLabAccountId
    ? accounts.find((account) => account.id === selectedLabAccountId) ?? null
    : null;

  const runtimeByAccountId: Record<string, SyncRuntimeAccountStatus> = useMemo(
    () => ({
      "lab-1": {
        profileId: "lab-1",
        phase: "syncing",
        phaseMessage: "Syncing 2 files",
        issueCode: null,
        issueMessage: null,
        issueActions: [],
        issuePath: null,
        issueSecondaryPath: null,
        updatedAt: new Date().toISOString(),
        inProgress: [
          {
            id: "lab-transfer-1",
            direction: "download",
            path: "Documents/ProjectPlan.docx",
            bytesDone: 3670016,
            bytesTotal: 4928307,
            startedAt: new Date(Date.now() - 90_000).toISOString(),
            updatedAt: new Date().toISOString(),
          },
          {
            id: "lab-transfer-2",
            direction: "upload",
            path: "Pictures/team-photo.png",
            bytesDone: 1048576,
            bytesTotal: 3145728,
            startedAt: new Date(Date.now() - 45_000).toISOString(),
            updatedAt: new Date().toISOString(),
          },
        ],
        recentCompleted: [
          {
            id: "lab-complete-1",
            direction: "download",
            path: "Reports/Q4-summary.pdf",
            bytesTotal: 1873920,
            finishedAt: new Date(Date.now() - 180_000).toISOString(),
            status: "completed",
            error: null,
          },
          {
            id: "lab-complete-2",
            direction: "upload",
            path: "Invoices/paid-2026-05.csv",
            bytesTotal: 48203,
            finishedAt: new Date(Date.now() - 110_000).toISOString(),
            status: "completed",
            error: null,
          },
        ],
        recentRetryWaiting: [],
        recentFailed: [],
      },
      "lab-2": {
        profileId: "lab-2",
        phase: "paused",
        phaseMessage: "Synchronization paused",
        issueCode: null,
        issueMessage: null,
        issueActions: [],
        issuePath: null,
        issueSecondaryPath: null,
        updatedAt: new Date().toISOString(),
        inProgress: [],
        recentCompleted: [],
        recentRetryWaiting: [],
        recentFailed: [],
      },
      "lab-3": {
        profileId: "lab-3",
        phase: "syncing",
        phaseMessage: "Syncing with retry warnings",
        issueCode: "conflict_detected",
        issueMessage: "Conflict detected. A safe backup was created.",
        issueActions: ["open_conflict", "open_sync_root", "retry_sync"],
        issuePath: "Designs/Brand/logo.ai",
        issueSecondaryPath: "Designs/Brand/logo.ai.safebackup",
        updatedAt: new Date().toISOString(),
        inProgress: [
          {
            id: "lab-transfer-3",
            direction: "upload",
            path: "Designs/Brand/logo.ai",
            bytesDone: 2867200,
            bytesTotal: 5242880,
            startedAt: new Date(Date.now() - 125_000).toISOString(),
            updatedAt: new Date().toISOString(),
          },
        ],
        recentCompleted: [],
        recentRetryWaiting: [
          {
            id: "lab-retry-1",
            direction: "download",
            path: "Shared/Assets/hero.psd",
            bytesTotal: 7340032,
            finishedAt: new Date(Date.now() + 15_000).toISOString(),
            status: "retry_waiting",
            error: "Retry queued after temporary transfer error",
          },
        ],
        recentFailed: [
          {
            id: "lab-failed-2",
            direction: "upload",
            path: "Designs/Brand/logo.ai",
            bytesTotal: 5242880,
            finishedAt: new Date(Date.now() - 20_000).toISOString(),
            status: "failed",
            error: "Temporary network interruption",
          },
          {
            id: "lab-failed-3",
            direction: "download",
            path: "Shared/Assets/hero.psd",
            bytesTotal: 7340032,
            finishedAt: new Date(Date.now() - 95_000).toISOString(),
            status: "failed",
            error: "Timed out while reading download stream",
          },
        ],
      },
      "lab-4": {
        profileId: "lab-4",
        phase: "paused",
        phaseMessage: "Synchronization paused",
        issueCode: null,
        issueMessage: null,
        issueActions: [],
        issuePath: null,
        issueSecondaryPath: null,
        updatedAt: new Date().toISOString(),
        inProgress: [],
        recentCompleted: [
          {
            id: "lab-complete-3",
            direction: "download",
            path: "Archive/2025/tax-summary.pdf",
            bytesTotal: 1153024,
            finishedAt: new Date(Date.now() - 300_000).toISOString(),
            status: "completed",
            error: null,
          },
        ],
        recentRetryWaiting: [],
        recentFailed: [],
      },
      "lab-5": {
        profileId: "lab-5",
        phase: "error",
        phaseMessage: "Sync error: authentication required",
        issueCode: "auth_required",
        issueMessage: "Authentication required",
        issueActions: ["reauthenticate", "retry_sync"],
        issuePath: null,
        issueSecondaryPath: null,
        updatedAt: new Date().toISOString(),
        inProgress: [],
        recentCompleted: [],
        recentRetryWaiting: [],
        recentFailed: [
          {
            id: "lab-failed-1",
            direction: "upload",
            path: "Notes/todo.txt",
            bytesTotal: 4096,
            finishedAt: new Date(Date.now() - 60_000).toISOString(),
            status: "failed",
            error: "Authentication required",
          },
        ],
      },
    }),
    []
  );

  const selectedRuntimeStatus = selectedLabAccount ? runtimeByAccountId[selectedLabAccount.id] ?? null : null;

  useEffect(() => {
    const syncFromHash = () => {
      const nextRoute = parseUiLabRoute(window.location.hash);
      setSelectedLabAccountId(nextRoute?.accountId ?? null);
      setSelectedLabAccountView(nextRoute?.view ?? "sync");
    };
    window.addEventListener("hashchange", syncFromHash);
    return () => {
      window.removeEventListener("hashchange", syncFromHash);
    };
  }, []);

  useEffect(() => {
    if (!selectedLabAccountId) {
      return;
    }
    if (!accounts.some((account) => account.id === selectedLabAccountId)) {
      navigateUiLab(null);
    }
  }, [accounts, selectedLabAccountId]);

  if (selectedLabAccount) {
    return (
      <AccountDetailPage
        account={selectedLabAccount}
        runtimeStatus={selectedRuntimeStatus}
        view={selectedLabAccountView}
        onBack={() => navigateUiLab(null)}
        onOpenSettings={(accountId) => navigateUiLab(accountId, "settings")}
        onOpenSync={(accountId) => navigateUiLab(accountId, "sync")}
        onSetAgentState={async (accountId, nextState) => {
          setLabAgentStateById((current) => ({ ...current, [accountId]: nextState }));
        }}
        onStartAuth={async () => null}
        onRename={async () => undefined}
        onSetSyncRoot={async () => undefined}
        onClearAuth={async () => undefined}
        onRemoveProfile={async () => undefined}
        onOpenSyncRootFolder={async () => undefined}
        onOpenItemFolder={async () => undefined}
        onReauthenticate={async () => null}
        onRetrySync={async () => undefined}
        onRetryFailedDownload={async () => undefined}
        onRetryAllFailedDownloads={async () => undefined}
        onConfirmLargeDelete={async () => undefined}
        onKeepCloudFiles={async () => undefined}
        onFetchLargeDeletePreview={async () => []}
        onExportLargeDeletePreview={async () => undefined}
      />
    );
  }

  return (
    <section class="page">
      <PageHeader title="UI Lab" back={{ href: "#/debug", label: "Back to debug tools", onBack }} />
      <div class="page-scroll">
        {accounts.length === 0 ? (
          <p>No accounts configured yet. Show setup call-to-action.</p>
        ) : (
          <div class="account-list">
            {accounts.map((account) => (
              <AccountCard
                key={account.id}
                account={account}
                runtimeStatus={runtimeByAccountId[account.id] ?? null}
                onOpenDetails={(accountId) => {
                  navigateUiLab(accountId, "sync");
                }}
                onSetAgentState={async (accountId, nextState) => {
                  setLabAgentStateById((current) => ({ ...current, [accountId]: nextState }));
                }}
                onOpenSyncRootFolder={async () => undefined}
              />
            ))}
          </div>
        )}
      </div>
    </section>
  );
}
