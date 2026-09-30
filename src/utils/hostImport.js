/**
 * Pure helpers for importing hosts from a JSON export or from ~/.ssh/config.
 * Kept out of the components so they can be tested; @vue/test-utils is not
 * installed, so component logic is otherwise untestable.
 */
import { stripMongoPassword } from './mongoUri.js'

export { stripMongoPassword }

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
  // An export written before credentials moved to the keyring still carries
  // them. Importing it verbatim would write plaintext passwords straight back
  // into the database and undo the migration.
  //
  // `mongo_local_uri` is still accepted so importing an older file is not
  // silently lossy: a host that had two connections gets its second one moved
  // to its own host on the next launch.
  for (const field of ['mongo_uri', 'mongo_local_uri']) {
    if (host[field]) host[field] = stripMongoPassword(host[field])
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

/**
 * Split a `ProxyJump` hop — `alias`, `user@host`, `host:port`, `[v6]:port` —
 * into its parts. `port` is null when not given.
 */
export function parseJumpSpec(spec) {
  let rest = String(spec).trim()
  let user = null
  const at = rest.lastIndexOf('@')
  if (at !== -1) {
    user = rest.slice(0, at)
    rest = rest.slice(at + 1)
  }
  let port = null
  const bracketed = rest.match(/^\[(.+)\](?::(\d+))?$/)
  if (bracketed) {
    rest = bracketed[1]
    port = bracketed[2] ? Number(bracketed[2]) : null
  } else {
    const colon = rest.match(/^([^:]+):(\d+)$/)
    if (colon) {
      rest = colon[1]
      port = Number(colon[2])
    }
  }
  return { user, host: rest, port }
}

const isSsh = h => !!h.host

/**
 * Link freshly imported hosts to their `ProxyJump` host.
 *
 * Runs after import because the rows a hop could name only have ids then.
 * `imported` is what `parse_ssh_config` returned for the selected hosts;
 * `hosts` is the host list after reloading.
 *
 * A hop matches a saved SSH host by name first (ssh_config aliases are how
 * people write it), then by address, honouring a port when the hop gives one.
 * Anything ambiguous or unsupported is reported rather than guessed:
 * connecting a host through the wrong bastion is worse than not linking it.
 *
 * Returns `{ links: [{ host, jumpId }], unresolved: [{ name, reason }] }`.
 */
export function linkProxyJumps(imported, hosts) {
  const newest = matches => matches.reduce((a, b) => (b.id > a.id ? b : a), matches[0])
  const links = []
  const unresolved = []

  for (const entry of imported) {
    if (!entry.proxy_jump) continue
    const row = newest(
      hosts.filter(
        h => isSsh(h) && h.name === entry.name && h.host === entry.host && h.port === entry.port,
      ),
    )
    if (!row) continue

    const spec = parseJumpSpec(entry.proxy_jump)
    const byName = hosts.filter(h => isSsh(h) && h.name === spec.host && h.id !== row.id)
    const byAddress = hosts.filter(
      h =>
        isSsh(h) &&
        h.host === spec.host &&
        h.id !== row.id &&
        (spec.port == null || h.port === spec.port) &&
        (spec.user == null || h.username === spec.user),
    )
    const candidates = byName.length ? byName : byAddress
    if (candidates.length === 0) {
      unresolved.push({ name: entry.name, reason: `no saved host matches ${entry.proxy_jump}` })
      continue
    }
    links.push({ host: row, jumpId: newest(candidates).id })
  }

  // One hop only: a jump host that is itself behind one, already or as part
  // of this import, would be refused at connect time.
  const jumped = new Set([
    ...hosts.filter(h => h.jump_host_id != null).map(h => h.id),
    ...links.map(l => l.host.id),
  ])
  return {
    links: links.filter(l => {
      if (!jumped.has(l.jumpId)) return true
      unresolved.push({ name: l.host.name, reason: 'its jump host has a jump host of its own' })
      return false
    }),
    unresolved,
  }
}

/**
 * The `NewHost` shape for rewriting an existing row with one field changed.
 * `update_host` writes every column, so every column must be carried.
 */
export function hostRowForUpdate(row, changes = {}) {
  return {
    name: row.name,
    host: row.host,
    port: row.port,
    username: row.username,
    auth_type: row.auth_type,
    key_path: row.key_path || null,
    group: row.group ?? null,
    favorite: row.favorite ?? null,
    mongo_uri: row.mongo_uri ?? null,
    mongo_local_uri: row.mongo_local_uri ?? null,
    redis_uri: row.redis_uri ?? null,
    redis_tunnel_host_id: row.redis_tunnel_host_id ?? null,
    jump_host_id: row.jump_host_id ?? null,
    ...changes,
  }
}
