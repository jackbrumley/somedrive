import { useEffect, useRef, useState } from "preact/hooks";
import type { SyncRuntimeAccountStatus } from "../types/somedrive";

interface TransferRates {
  downloadBytesPerSecond: number;
  uploadBytesPerSecond: number;
}

const SMOOTHING_PREVIOUS_WEIGHT = 0.55;

function smooth(current: number, next: number): number {
  return current <= 0 ? next : current * SMOOTHING_PREVIOUS_WEIGHT + next * (1 - SMOOTHING_PREVIOUS_WEIGHT);
}

/** Derives smoothed download and upload speeds from successive runtime status byte totals. */
export function useTransferRates(runtimeStatus: SyncRuntimeAccountStatus | null): TransferRates {
  const previousSampleRef = useRef<{ timestampMs: number; downloadedBytes: number; uploadedBytes: number } | null>(null);
  const [rates, setRates] = useState<TransferRates>({ downloadBytesPerSecond: 0, uploadBytesPerSecond: 0 });

  useEffect(() => {
    if (!runtimeStatus || runtimeStatus.phase === "paused") {
      previousSampleRef.current = null;
      setRates({ downloadBytesPerSecond: 0, uploadBytesPerSecond: 0 });
      return;
    }
    const parsedTimestampMs = Date.parse(runtimeStatus.updatedAt);
    const timestampMs = Number.isFinite(parsedTimestampMs) ? parsedTimestampMs : Date.now();
    const downloadedBytes =
      (runtimeStatus.remoteDownloadCompletedBytesTotal ?? 0) + (runtimeStatus.remoteDownloadInFlightBytesDone ?? 0);
    const uploadedBytes = (runtimeStatus.uploadCompletedBytesTotal ?? 0) + (runtimeStatus.uploadInFlightBytesDone ?? 0);
    const previousSample = previousSampleRef.current;
    if (previousSample && timestampMs > previousSample.timestampMs) {
      const elapsedSeconds = (timestampMs - previousSample.timestampMs) / 1000;
      const nextDownloadRate = Math.max(0, (downloadedBytes - previousSample.downloadedBytes) / elapsedSeconds);
      const nextUploadRate = Math.max(0, (uploadedBytes - previousSample.uploadedBytes) / elapsedSeconds);
      setRates((current) => ({
        downloadBytesPerSecond: smooth(current.downloadBytesPerSecond, nextDownloadRate),
        uploadBytesPerSecond: smooth(current.uploadBytesPerSecond, nextUploadRate),
      }));
    }
    previousSampleRef.current = { timestampMs, downloadedBytes, uploadedBytes };
  }, [runtimeStatus]);

  return rates;
}
