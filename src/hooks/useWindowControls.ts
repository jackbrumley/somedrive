import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useRef, useState } from "preact/hooks";
import type { ToastType } from "../types/somedrive";

type ResizeDirection = "East" | "North" | "NorthEast" | "NorthWest" | "South" | "SouthEast" | "SouthWest" | "West";

/// Where the backend parked the main window (`tray::WindowParking`).
type WindowParking = "tray" | "taskbar";

export function useWindowControls(showToast: (message: string, type?: ToastType) => void) {
  const [isMaximized, setIsMaximized] = useState(false);
  const trayFallbackNotifiedRef = useRef(false);

  const notifyIfParkedInTaskbar = (parking: WindowParking | null) => {
    if (parking === "taskbar" && !trayFallbackNotifiedRef.current) {
      trayFallbackNotifiedRef.current = true;
      showToast("System tray is unavailable on this desktop. Minimized to taskbar instead.", "info");
    }
  };

  useEffect(() => {
    const appWindow = getCurrentWindow();

    const syncMaximizeState = async () => {
      try {
        setIsMaximized(await appWindow.isMaximized());
      } catch {
        // no-op
      }
    };

    syncMaximizeState();
    const unlistenPromise = appWindow.onResized(() => {
      syncMaximizeState();
    });

    return () => {
      unlistenPromise.then((unlisten) => unlisten()).catch(() => {
        // no-op
      });
    };
  }, []);

  // Parks the window: hidden to the tray when one exists, else minimized.
  const minimize = async () => {
    try {
      notifyIfParkedInTaskbar(await invoke<WindowParking>("minimize_to_tray_or_taskbar"));
    } catch (error) {
      console.error("Failed to park window:", error);
      await getCurrentWindow().minimize();
    }
  };

  const toggleMaximize = async () => {
    try {
      const appWindow = getCurrentWindow();
      if (await appWindow.isMaximized()) {
        await appWindow.unmaximize();
      } else {
        await appWindow.maximize();
      }
      setIsMaximized(await appWindow.isMaximized());
    } catch (error) {
      console.error("Failed to toggle maximize:", error);
    }
  };

  // Honors the close-to-tray setting: parks the window, or quits SomeDrive
  // (which finishes in-flight sync first).
  const close = async () => {
    try {
      notifyIfParkedInTaskbar(await invoke<WindowParking | null>("close_main_window"));
    } catch (error) {
      console.error("Failed to close window:", error);
      await getCurrentWindow().close();
    }
  };

  const handleTitleBarMouseDown = async (event: MouseEvent) => {
    if (event.detail > 1) {
      event.preventDefault();
      return;
    }

    if (event.button === 0 && !(event.target as HTMLElement).closest("button, input, select, a, [data-no-drag='true']")) {
      try {
        await getCurrentWindow().startDragging();
      } catch (error) {
        console.error("Failed to start dragging:", error);
      }
    }
  };

  const handleTitleBarDoubleClick = async (event: MouseEvent) => {
    if (!(event.target as HTMLElement).closest("button, input, select, a, [data-no-drag='true']")) {
      event.preventDefault();
      await toggleMaximize();
    }
  };

  const handleResizeMouseDown = (direction: ResizeDirection) => async (event: MouseEvent) => {
    if (event.buttons !== 1) {
      return;
    }

    if (isMaximized) {
      return;
    }

    event.preventDefault();
    event.stopPropagation();

    try {
      await getCurrentWindow().startResizeDragging(direction);
    } catch {
      // no-op when resize dragging is unavailable
    }
  };

  return {
    isMaximized,
    minimize,
    toggleMaximize,
    close,
    handleTitleBarMouseDown,
    handleTitleBarDoubleClick,
    handleResizeMouseDown,
  };
}
