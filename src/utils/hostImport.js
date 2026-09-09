/**
 * Pure helpers for importing hosts from a JSON export or from ~/.ssh/config.
 * Kept out of the components so they can be tested; @vue/test-utils is not
 * installed, so component logic is otherwise untestable.
 */

/** The only fields the backend accepts. Anything else is dropped. */
const HOST_FIELDS = [
  'name',
  'host',
  'port',
  'username',
  'auth_type',
  'key_path',
  'group',
  'favorite',
  'mongo_uri',
  'mongo_local_uri',
]

/**
 * Read an exported hosts file. Accepts either a bare array, which is what
 * export has always written, or a `{ hosts: [...] }` envelope.
 * Throws with a readable message rather than surfacing a serde error.
 */
export function parseHostsFile(text) {
  let parsed
  try {
    parsed = JSON.parse(text)
  } catch {
    throw new Error('This file is not valid JSON')
  }

  const hosts = Array.isArray(parsed) ? parsed : parsed?.hosts
  if (!Array.isArray(hosts)) {
    throw new Error('This does not look like a TermDrop host export')
  }
  if (hosts.some((h) => h === null || typeof h !== 'object' || Array.isArray(h))) {
    throw new Error('The file contains an entry that is not a host')
  }
  return hosts
}

/**
 * Coerce one parsed record into the shape the backend expects, keeping only
 * known fields. Dropping the rest means a stray `password` key in a
 * hand-edited file can never reach the database.
 */
export function normalizeImportHost(raw) {
  const host = {}
  for (const field of HOST_FIELDS) {
    host[field] = raw[field]
  }

  host.name = typeof host.name === 'string' ? host.name.trim() : ''
  host.host = typeof host.host === 'string' ? host.host.trim() : ''
  host.username = typeof host.username === 'string' ? host.username.trim() : ''
  host.auth_type = host.auth_type === 'key' ? 'key' : 'password'
  host.port = Number.isFinite(Number(host.port)) ? Number(host.port) : 22
  host.favorite = host.favorite ? 1 : 0
  for (const field of ['key_path', 'group', 'mongo_uri', 'mongo_local_uri']) {
    host[field] = host[field] == null ? null : String(host[field])
  }
  return host
}

const plural = (n, word) => `${n} ${word}${n === 1 ? '' : 's'}`

/**
 * A one-line report of what an import actually did. Zero counts are omitted,
 * so a clean import reads "Imported 3 hosts" rather than listing zeroes.
 */
export function summarizeImport({ added = 0, replaced = 0, failed = [], skipped = 0 } = {}) {
  const parts = []
  if (added) parts.push(`Imported ${plural(added, 'host')}`)
  if (replaced) parts.push(`replaced ${plural(replaced, 'host')}`)
  if (skipped) parts.push(`skipped ${skipped}`)
  if (failed.length) parts.push(`${plural(failed.length, 'failure')}`)

  if (parts.length === 0) return { message: 'Nothing to import', type: 'info' }

  let message = parts.join(', ')
  if (failed.length) {
    // Name the first failure so the message is actionable without a log.
    message += `: ${failed[0].name || 'unnamed'} (${failed[0].reason})`
    if (failed.length > 1) message += ` and ${failed.length - 1} more`
  }
  return { message, type: failed.length ? 'warning' : 'success' }
}
