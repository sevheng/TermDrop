/**
 * Human-readable byte count: "512 B", "1.5 KB", "2.0 MB", "1.0 GB".
 */
export function formatBytes(bytes) {
  if (bytes < 1024) return bytes + ' B'
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB'
  if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + ' MB'
  return (bytes / (1024 * 1024 * 1024)).toFixed(1) + ' GB'
}

/**
 * Signed transfer rate. Picks the unit from the magnitude so negative
 * values (e.g. a counter reset) keep their sign: "-2.0 KB/s".
 */
export function formatRate(bytesPerSec) {
  const abs = Math.abs(bytesPerSec)
  if (abs < 1024) return bytesPerSec.toFixed(0) + ' B/s'
  if (abs < 1024 * 1024) return (bytesPerSec / 1024).toFixed(1) + ' KB/s'
  return (bytesPerSec / (1024 * 1024)).toFixed(1) + ' MB/s'
}

/**
 * Unsigned transfer rate for values that are never negative. Differs from
 * formatRate only for negative input, where it stays in "B/s".
 */
export function formatSpeed(bytesPerSec) {
  if (bytesPerSec < 1024) return bytesPerSec.toFixed(0) + ' B/s'
  if (bytesPerSec < 1024 * 1024) return (bytesPerSec / 1024).toFixed(1) + ' KB/s'
  return (bytesPerSec / (1024 * 1024)).toFixed(1) + ' MB/s'
}
