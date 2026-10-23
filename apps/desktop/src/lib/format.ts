const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatBytes(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value.toFixed(unit === 0 || value >= 100 ? 0 : 1)} ${UNITS[unit]}`;
}

export function formatSpeed(bps: number): string {
  return `${formatBytes(bps)}/s`;
}

export function formatEta(secs: number | null): string {
  if (secs === null) return "estimating…";
  if (secs < 60) return `${secs}s left`;
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m}m ${secs % 60}s left`;
  return `${Math.floor(m / 60)}h ${m % 60}m left`;
}

/** Last path component, for either separator. */
export function basename(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}
