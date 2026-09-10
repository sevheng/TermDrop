/**
 * Separating a MongoDB URI from its password.
 *
 * Mirrors split_mongo_password in src-tauri/src/mongodb.rs: the last `@` in the
 * authority separates userinfo from the host list, and the first `:` within the
 * userinfo separates user from password. The password is left percent-encoded,
 * exactly as it appeared.
 */

/** `{ uri, password }` — `password` is null when the URI carries none. */
export function splitMongoUri(uri) {
  const value = uri == null ? '' : String(uri)
  const schemeMatch = /^mongodb(\+srv)?:\/\//.exec(value)
  if (!schemeMatch) return { uri: value, password: null }

  const start = schemeMatch[0].length
  const rest = value.slice(start)
  const authorityLen = (() => {
    const idx = rest.search(/[/?#]/)
    return idx === -1 ? rest.length : idx
  })()

  const at = rest.slice(0, authorityLen).lastIndexOf('@')
  if (at === -1) return { uri: value, password: null }

  const userinfo = rest.slice(0, at)
  const colon = userinfo.indexOf(':')
  if (colon === -1) return { uri: value, password: null }

  const password = userinfo.slice(colon + 1)
  if (!password) return { uri: value, password: null }

  return {
    uri: value.slice(0, start) + userinfo.slice(0, colon) + value.slice(start + at),
    password,
  }
}

/** The URI without its password, leaving the username in place. */
export function stripMongoPassword(uri) {
  return splitMongoUri(uri).uri
}
