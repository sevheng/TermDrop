/**
 * Rendering a MongoDB URI for display.
 *
 * A connection string is never shown verbatim: even after credentials moved to
 * the keyring, a URI in a tooltip or a list row is noise at best and a leaked
 * password at worst for a row written before the migration.
 */

/**
 * `host:port` for a connection string, or a truncated form when it cannot be
 * parsed. Never includes credentials.
 */
export function mongoDisplayUri(uri) {
  const value = (uri || '').trim()
  if (!value) return ''

  const isSrv = value.startsWith('mongodb+srv://')
  try {
    // URL cannot parse the mongodb scheme; swap it for one it knows. Any
    // userinfo is dropped rather than rendered.
    const url = new URL(value.replace(/^mongodb(\+srv)?:\/\//, 'http://'))
    if (isSrv) return `mongodb+srv://${url.hostname}`
    return `mongodb://${url.hostname}${url.port ? ':' + url.port : ''}`
  } catch {
    return stripCredentials(value)
  }
}

/**
 * Remove any `user:password@` from a URI that could not be parsed, so the
 * fallback path cannot render a credential either.
 */
export function stripCredentials(uri) {
  const withoutUserinfo = uri.replace(/^(mongodb(?:\+srv)?:\/\/)[^/@]*@/, '$1')
  return withoutUserinfo.length > 35 ? withoutUserinfo.slice(0, 35) + '…' : withoutUserinfo
}
