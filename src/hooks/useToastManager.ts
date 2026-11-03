import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "preact/hooks";
import type { ToastMessage, ToastType } from "../types/somedrive";

const toastDurationMs: Record<ToastType, number> = {
  info: 3000,
  success: 3000,
  error: 10000,
};

export function useToastManager() {
  const [toast, setToast] = useState<ToastMessage | null>(null);
  const timeoutRef = useRef<number | null>(null);
  const remainingRef = useRef(0);
  const startedAtRef = useRef(0);

  const clearTimer = () => {
    if (timeoutRef.current !== null) {
      window.clearTimeout(timeoutRef.current);
      timeoutRef.current = null;
    }
  };

  const dismissToast = () => {
    clearTimer();
    setToast(null);
  };

  const scheduleDismiss = (durationMs: number) => {
    clearTimer();
    startedAtRef.current = Date.now();
    remainingRef.current = durationMs;
    timeoutRef.current = window.setTimeout(() => {
      setToast(null);
      timeoutRef.current = null;
    }, durationMs);
  };

  const logToastShown = (message: string, type: ToastType, durationMs: number) => {
    const normalizedMessage = message.replace(/\s+/g, " ").replace(/"/g, "\\\"").trim();
    void invoke("log_ui_event", {
      message: `toast type=${type} duration_ms=${durationMs} message="${normalizedMessage}"`,
    }).catch(() => {
      // no-op
    });
  };

  // Durations follow the App Standard: errors stay long enough to read and
  // act on, confirmations get out of the way.
  const showToast = (message: string, type: ToastType = "info") => {
    const durationMs = toastDurationMs[type];
    setToast({
      id: Date.now(),
      message,
      type,
      durationMs,
    });
    logToastShown(message, type, durationMs);
    scheduleDismiss(durationMs);
  };

  const pauseToast = () => {
    if (!toast || timeoutRef.current === null) {
      return;
    }
    const elapsed = Date.now() - startedAtRef.current;
    remainingRef.current = Math.max(0, remainingRef.current - elapsed);
    clearTimer();
  };

  const resumeToast = () => {
    if (!toast || timeoutRef.current !== null) {
      return;
    }
    scheduleDismiss(Math.max(250, remainingRef.current));
  };

  useEffect(() => () => clearTimer(), []);

  return {
    toast,
    showToast,
    dismissToast,
    pauseToast,
    resumeToast,
  };
}
