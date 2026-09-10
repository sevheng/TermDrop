/**
 * What a host row represents.
 *
 * There is no type column: a row with a datastore URI and no SSH host *is*
 * that datastore. The predicate used to be written inline in four places
 * (`!!(host.mongo_uri && !host.host)`), which is exactly the kind of thing
 * that goes out of step when a third kind arrives.
 */

export const HOST_KIND = {
  SSH: 'ssh',
  MONGODB: 'mongodb',
  REDIS: 'redis',
}

/**
 * A host's kind. Redis is checked first so a row that somehow carries both
 * URIs resolves the same way every time rather than depending on key order.
 */
export function hostKind(host) {
  if (!host) return HOST_KIND.SSH
  if (!host.host) {
    if (host.redis_uri) return HOST_KIND.REDIS
    if (host.mongo_uri) return HOST_KIND.MONGODB
  }
  return HOST_KIND.SSH
}

export function isMongoOnlyHost(host) {
  return hostKind(host) === HOST_KIND.MONGODB
}

export function isRedisOnlyHost(host) {
  return hostKind(host) === HOST_KIND.REDIS
}

/** A datastore host opens a panel tab rather than an SSH session. */
export function isDatastoreHost(host) {
  return hostKind(host) !== HOST_KIND.SSH
}

/** The verb for the context menu and the double-click action. */
export function activateVerb(host) {
  return isDatastoreHost(host) ? 'Open' : 'Connect'
}

/**
 * The colour a host's icon takes.
 *
 * The row drew every kind in `text-ink-3`, so a Redis host and an SSH host
 * looked identical until you read the address underneath — even though the
 * brand tokens existed and the sidebar's Add menu already used them.
 */
export function hostKindIconClass(host) {
  return {
    [HOST_KIND.REDIS]: 'text-redis',
    [HOST_KIND.MONGODB]: 'text-mongo',
  }[hostKind(host)] || 'text-ink-3'
}
