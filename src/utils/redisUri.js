/**
 * Reading and writing `redis://` URIs.
 *
 * Mirrors `src-tauri/src/uri.rs`: the last `@` in the authority separates
 * userinfo from host, and the first `:` inside the userinfo separates user from
 * password. The password stays percent-encoded exactly as it appeared, so
 * putting it back is a pure splice.
 *
 * The form worth being careful about is `redis://:password@host` — Redis's
 * usual one, with no username at all. Stripping it leaves `redis://@host`,
 * which still carries credentials even though the username is empty.
 */

const SCHEME_RE = /^rediss?:\/\//i

export function isRedisUri(uri) {
  return SCHEME_RE.test(String(uri ?? '').trim())
}

export function isTlsRedisUri(uri) {
  return /^rediss:\/\//i.test(String(uri ?? '').trim())
}

/** The byte offsets of the authority, i.e. between `://` and the path. */
function authorityRange(uri) {
  const m = SCHEME_RE.exec(uri)
  if (!m) return null
  const start = m[0].length
  const rest = uri.slice(start)
  const idx = rest.search(/[/?#]/)
  return { start, end: idx === -1 ? uri.length : start + idx }
}

/** `{ uri, password }` — `password` is null when the URI carries none. */
export function splitRedisUri(uri) {
  const value = uri == null ? '' : String(uri)
  const range = authorityRange(value)
  if (!range) return { uri: value, password: null }

  const authority = value.slice(range.start, range.end)
  const at = authority.lastIndexOf('@')
  if (at === -1) return { uri: value, password: null }

  const userinfo = authority.slice(0, at)
  const colon = userinfo.indexOf(':')
  if (colon === -1) return { uri: value, password: null }

  const password = userinfo.slice(colon + 1)
  if (!password) return { uri: value, password: null }

  return {
    uri:
      value.slice(0, range.start) +
      userinfo.slice(0, colon) +
      value.slice(range.start + at),
    password,
  }
}

/** The URI without its password, leaving any username in place. */
export function stripRedisPassword(uri) {
  return splitRedisUri(uri).uri
}

/** A URI with its credentials masked, for display. */
export function redisDisplayUri(uri) {
  const value = uri == null ? '' : String(uri)
  const range = authorityRange(value)
  if (!range) return value

  const authority = value.slice(range.start, range.end)
  const at = authority.lastIndexOf('@')
  if (at === -1) return value

  const masked = authority.slice(0, at).includes(':') ? '***:***' : '***'
  return value.slice(0, range.start) + masked + value.slice(range.start + at)
}

/** Pull a URI apart into the fields the connection form shows. */
export function parseRedisUri(uri) {
  const value = (uri == null ? '' : String(uri)).trim()
  const empty = {
    scheme: 'redis',
    host: '',
    port: '',
    username: '',
    password: '',
    database: '',
  }
  const range = authorityRange(value)
  if (!range) return empty

  const scheme = value.slice(0, value.indexOf(':')).toLowerCase()
  const authority = value.slice(range.start, range.end)
  const at = authority.lastIndexOf('@')
  const userinfo = at === -1 ? '' : authority.slice(0, at)
  const hostport = at === -1 ? authority : authority.slice(at + 1)

  const colon = userinfo.indexOf(':')
  const username = colon === -1 ? userinfo : userinfo.slice(0, colon)
  const password = colon === -1 ? '' : userinfo.slice(colon + 1)

  // An IPv6 literal is bracketed, and its colons are not port separators.
  let host = hostport
  let port = ''
  if (hostport.startsWith('[')) {
    const close = hostport.indexOf(']')
    if (close !== -1) {
      host = hostport.slice(0, close + 1)
      const tail = hostport.slice(close + 1)
      port = tail.startsWith(':') ? tail.slice(1) : ''
    }
  } else {
    const portColon = hostport.lastIndexOf(':')
    if (portColon !== -1) {
      host = hostport.slice(0, portColon)
      port = hostport.slice(portColon + 1)
    }
  }

  const path = value.slice(range.end).replace(/^\//, '').split(/[?#]/)[0]

  return { scheme, host, port, username, password, database: path }
}

/**
 * Build a URI from the form's fields.
 *
 * Defaults are left out rather than spelled: a URI reading
 * `redis://localhost:6379/0` and one reading `redis://localhost` mean the same
 * thing, and the shorter one is what the user typed.
 */
export function buildRedisUri({
  scheme = 'redis',
  host = '',
  port = '',
  username = '',
  password = '',
  database = '',
} = {}) {
  const trimmedHost = String(host).trim()
  if (!trimmedHost) return ''

  let userinfo = ''
  if (username || password) {
    userinfo = `${username}${password ? `:${password}` : ''}@`
  }

  const p = String(port).trim()
  const hostport = p && p !== '6379' ? `${trimmedHost}:${p}` : trimmedHost

  const db = String(database).trim()
  const path = db && db !== '0' ? `/${db}` : ''

  return `${scheme}://${userinfo}${hostport}${path}`
}
