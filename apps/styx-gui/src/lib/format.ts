const UNITS = ["B", "KiB", "MiB", "GiB"] as const;

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return "0 B";
  }

  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }

  return unit === 0 ? `${Math.floor(value)} ${UNITS[unit]}` : `${value.toFixed(1)} ${UNITS[unit]}`;
}

export function formatRate(bytesPerSecond: number): string {
  return `${formatBytes(bytesPerSecond)}/s`;
}

export function formatPercent(progress: number): string {
  const value = Number.isFinite(progress) ? progress : 0;
  return `${(Math.min(1, Math.max(0, value)) * 100).toFixed(1)}%`;
}
