import { useEffect, useRef, useState } from "preact/hooks";
import type { SyncRuntimeAccountStatus } from "../types/somedrive";

export interface TransferRates {
  downloadBytesPerSecond: number;
  uploadBytesPerSecond: number;
  downloadFilesPerSecond: number;
  uploadFilesPerSecond: number;
}

const NO_RATES: TransferRates = {
  downloadBytesPerSecond: 0,
  uploadBytesPerSecond: 0,
  downloadFilesPerSecond: 0,
  uploadFilesPerSecond: 0,
};

const SMOOTHING_PREVIOUS_WEIGHT = 0.55;
/** Files finish one at a time, in bursts: their rate is taken over this
 *  window instead of between two status updates, so it does not jump
 *  between zero and double. */
const FILE_RATE_WINDOW_MS = 10_000;

interface Sample {
  timestampMs: number;
  downloadedBytes: number;
  uploadedBytes: number;
  downloadedFiles: number;
  uploadedFiles: number;
}

function smooth(current: number, next: number): number {
  return current <= 0 ? next : current * SMOOTHING_PREVIOUS_WEIGHT + next * (1 - SMOOTHING_PREVIOUS_WEIGHT);
}

function sampleOf(runtimeStatus: SyncRuntimeAccountStatus, timestampMs: number): Sample {
  return {
    timestampMs,
    downloadedBytes:
      (runtimeStatus.remoteDownloadCompletedBytesTotal ?? 0) + (runtimeStatus.remoteDownloadInFlightBytesDone ?? 0),
    uploadedBytes: (runtimeStatus.uploadCompletedBytesTotal ?? 0) + (runtimeStatus.uploadInFlightBytesDone ?? 0),
    downloadedFiles: runtimeStatus.remoteDownloadCompletedTotal ?? 0,
    uploadedFiles: runtimeStatus.uploadCompletedTotal ?? 0,
  };
}

/**
 * Derives transfer speeds from successive runtime status totals: bytes per
 * second (smoothed between updates) and files per second (over the last
 * `FILE_RATE_WINDOW_MS`), for downloads and uploads.
 */
export function useTransferRates(runtimeStatus: SyncRuntimeAccountStatus | null): TransferRates {
  const samplesRef = useRef<Sample[]>([]);
  const [rates, setRates] = useState<TransferRates>(NO_RATES);

  useEffect(() => {
    if (!runtimeStatus || runtimeStatus.phase === "paused") {
      samplesRef.current = [];
      setRates(NO_RATES);
      return;
    }
    const parsedTimestampMs = Date.parse(runtimeStatus.updatedAt);
    const timestampMs = Number.isFinite(parsedTimestampMs) ? parsedTimestampMs : Date.now();
    const sample = sampleOf(runtimeStatus, timestampMs);
    const samples = samplesRef.current;
    const previous = samples.length > 0 ? samples[samples.length - 1] : undefined;
    if (previous && timestampMs <= previous.timestampMs) {
      return;
    }
    samples.push(sample);
    while (samples.length > 2 && timestampMs - samples[1].timestampMs >= FILE_RATE_WINDOW_MS) {
      samples.shift();
    }
    if (!previous) {
      return;
    }
    const stepSeconds = (timestampMs - previous.timestampMs) / 1000;
    const oldest = samples[0];
    const windowSeconds = (timestampMs - oldest.timestampMs) / 1000;
    setRates((current) => ({
      downloadBytesPerSecond: smooth(
        current.downloadBytesPerSecond,
        Math.max(0, (sample.downloadedBytes - previous.downloadedBytes) / stepSeconds)
      ),
      uploadBytesPerSecond: smooth(
        current.uploadBytesPerSecond,
        Math.max(0, (sample.uploadedBytes - previous.uploadedBytes) / stepSeconds)
      ),
      downloadFilesPerSecond: Math.max(0, (sample.downloadedFiles - oldest.downloadedFiles) / windowSeconds),
      uploadFilesPerSecond: Math.max(0, (sample.uploadedFiles - oldest.uploadedFiles) / windowSeconds),
    }));
  }, [runtimeStatus]);

  return rates;
}
