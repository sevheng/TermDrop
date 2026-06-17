const DEFAULT_PORT = 27017
const DEFAULT_AUTH_SOURCE = 'admin'

/**
 * Parse a MongoDB connection URI into structured fields.
 * Returns { mode: 'form', host, port, username, password, database, authSource, options }
 * or { mode: 'uri', uri } for replica-set / unparsable URIs.
 */
export function parseMongoUri(uri) {
  if (!uri || typeof uri !== 'string') {
    return {
      mode: 'form',
      host: '',
      port: DEFAULT_PORT,
      username: '',
      password: '',
      database: '',
      authSource: DEFAULT_AUTH_SOURCE,
      options: '',
    }
  }

  // Replica-set URIs with multiple hosts cannot be represented by the single-host form.
  if (/mongodb:\/\/[^/]*,/.test(uri)) {
    return { mode: 'uri', uri }
  }

  try {
    const url = new URL(uri)

    const database = url.pathname ? decodeURIComponent(url.pathname.replace(/^\//, '')) : ''

    const authSource = url.searchParams.get('authSource') || DEFAULT_AUTH_SOURCE
    url.searchParams.delete('authSource')

    // Remaining query options, excluding authSource.
    const options = url.searchParams.toString()

    return {
      mode: 'form',
      host: url.hostname,
      port: url.port ? Number(url.port) : DEFAULT_PORT,
      username: decodeURIComponent(url.username || ''),
      password: decodeURIComponent(url.password || ''),
      database,
      authSource,
      options,
    }
  } catch {
    return { mode: 'uri', uri }
  }
}

/**
 * Build a MongoDB connection URI from structured fields.
 */
export function buildMongoUri({
  host,
  port,
  username,
  password,
  database,
  authSource,
  options,
}) {
  let uri = 'mongodb://'

  const user = (username || '').trim()
  const pass = (password || '').trim()

  if (user) {
    uri += encodeURIComponent(user)
    if (pass) {
      uri += ':' + encodeURIComponent(pass)
    }
    uri += '@'
  }

  let hostTrimmed = (host || '').trim()
  if (hostTrimmed.startsWith('[') && hostTrimmed.endsWith(']')) {
    hostTrimmed = hostTrimmed.slice(1, -1)
  }

  if (hostTrimmed.includes(':')) {
    uri += `[${hostTrimmed}]`
  } else {
    uri += hostTrimmed
  }

  const portNum = Number(port)
  if (portNum && portNum !== DEFAULT_PORT) {
    uri += ':' + portNum
  }

  const db = (database || '').trim()
  if (db) {
    uri += '/' + encodeURIComponent(db)
  }

  const params = new URLSearchParams()

  const authSrc = (authSource || '').trim()
  if (authSrc && authSrc !== DEFAULT_AUTH_SOURCE) {
    params.set('authSource', authSrc)
  }

  const opts = (options || '').trim()
  if (opts) {
    const extra = new URLSearchParams(opts)
    for (const [key, value] of extra) {
      if (key !== 'authSource') {
        params.set(key, value)
      }
    }
  }

  const query = params.toString()
  if (query) {
    uri += '?' + query
  }

  return uri
}
