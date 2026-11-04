export function formatBytes(value: number | null): string {
  if (value === null) {
    return "0 B";
  }
  if (value < 1024) {
    return `${value} B`;
  }
  if (value < 1024 * 1024) {
    return `${(value / 1024).toFixed(1)} KB`;
  }
  if (value < 1024 * 1024 * 1024) {
    return `${(value / (1024 * 1024)).toFixed(1)} MB`;
  }
  return `${(value / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

export function formatTransferRate(bytesPerSecond: number): string {
  if (!Number.isFinite(bytesPerSecond) || bytesPerSecond <= 0) {
    return "0 MB/s";
  }
  const megabytesPerSecond = bytesPerSecond / (1024 * 1024);
  return `${megabytesPerSecond >= 10 ? megabytesPerSecond.toFixed(1) : megabytesPerSecond.toFixed(2)} MB/s`;
}

export function percentOf(done: number, total: number | null): number | null {
  if (!total || total <= 0) {
    return null;
  }
  return Math.min(100, Math.max(0, (done / total) * 100));
}

/** Splits "Folder/Sub/file.txt" into the file name and its folder. */
export function splitSyncPath(path: string): { name: string; folder: string } {
  const separatorIndex = path.lastIndexOf("/");
  if (separatorIndex < 0) {
    return { name: path, folder: "" };
  }
  return { name: path.slice(separatorIndex + 1), folder: path.slice(0, separatorIndex) };
}

export function formatClockTime(value: string | undefined): string {
  if (!value) {
    return "—";
  }
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? "—" : parsed.toLocaleTimeString();
}
