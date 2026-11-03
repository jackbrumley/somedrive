import { AppShell } from "./components/shell/AppShell";
import { ToastHost } from "./components/toast/ToastHost";
import { UpdateModal } from "./components/updates/UpdateModal";
import { useAppRuntime } from "./hooks/useAppRuntime";
import { useToastManager } from "./hooks/useToastManager";
import { useUiInteractionLogger } from "./hooks/useUiInteractionLogger";
import { useUpdates } from "./hooks/useUpdates";
import { AppWorkspace } from "./layout/AppWorkspace";

function App() {
  const { toast, showToast, dismissToast, pauseToast, resumeToast } = useToastManager();
  const runtime = useAppRuntime({ showToast });
  const updates = useUpdates({ showToast });
  const handleGlobalClickCapture = useUiInteractionLogger(runtime.routeState);

  return (
    <div onClickCapture={handleGlobalClickCapture}>
      <AppShell
        page={runtime.routeState.page}
        onGoHome={runtime.goHome}
        onGoSettings={runtime.goSettings}
        syncingCount={runtime.syncingCount}
        pausedCount={runtime.pausedCount}
        onPauseAll={runtime.pauseAllAccounts}
        onResumeAll={runtime.resumeAllAccounts}
      >
        <AppWorkspace runtime={runtime} updates={updates} />
        {updates.showUpdateModal && updates.updateResult && (
          <UpdateModal
            updateResult={updates.updateResult}
            installStage={updates.installStage}
            installError={updates.installError}
            onUpdateNow={() => void updates.installUpdate()}
            onViewReleaseNotes={updates.openReleaseNotes}
            onLater={updates.closeUpdateModal}
          />
        )}
        <ToastHost toast={toast} onDismiss={dismissToast} onPause={pauseToast} onResume={resumeToast} />
      </AppShell>
    </div>
  );
}

export default App;
