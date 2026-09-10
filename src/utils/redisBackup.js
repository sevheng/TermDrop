/**
 * Naming and summarizing Redis backups.
 *
 * Kept out of the components so they can be tested.
 */

/** Pad to two digits without pulling in a date library. */
const pad = n => String(n).padStart(2, '0')

/**
 * A filename that says what the file holds.
 *
 * The date is local and the pattern is included, because the two questions
 * asked of a folder of backups are "when" and "what was in it".
 */
export function defaultBackupName(hostName, db, pattern) {
  const now = new Date()
  const stamp = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`
  const safeName = String(hostName || 'redis')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
  const scope = pattern ? `-${pattern.replace(/[^a-zA-Z0-9]+/g, '_')}` : ''
  return `${safeName || 'redis'}-db${db}${scope}-${stamp}.tdredis`
}

/**
 * What to tell the user when an export or import finishes.
 *
 * Every non-zero count is reported. A restore that silently skipped 12,000
 * existing keys "succeeded", and saying only that would be misleading — the
 * whole reason these operations return a summary rather than nothing.
 */
export function summarizeOp(summary, verb) {
  if (!summary) return { message: `${verb}.`, type: 'success' }

  const parts = [`${verb} ${summary.written.toLocaleString()} keys`]
  if (summary.skipped_existing > 0) {
    parts.push(`${summary.skipped_existing.toLocaleString()} already existed and were left alone`)
  }
  if (summary.skipped_big?.length > 0) {
    parts.push(`${summary.skipped_big.length} too large to copy`)
  }
  if (summary.vanished > 0) {
    parts.push(`${summary.vanished.toLocaleString()} expired while running`)
  }
  if (summary.failed?.length > 0) {
    parts.push(`${summary.failed.length} failed`)
  }

  return {
    message: parts.join('; '),
    // A failure is a warning, not a success, even when most keys went through.
    type: summary.failed?.length > 0 ? 'warning' : 'success',
  }
}

/**
 * The warning to show before restoring, or '' when there is nothing to say.
 *
 * The backend refuses an impossible restore outright; this is the softer case,
 * where the file simply came from somewhere else.
 */
export function restoreWarning(header, serverInfo) {
  if (!header || !serverInfo) return ''
  if (header.redis_version && serverInfo.version && header.redis_version !== serverInfo.version) {
    return `This backup came from Redis ${header.redis_version}; this server runs ${serverInfo.version}.`
  }
  return ''
}
