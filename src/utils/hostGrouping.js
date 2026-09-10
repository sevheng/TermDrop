/**
 * Filtering and grouping the host list.
 *
 * Extracted from HostSidebar because none of it was reachable from a test
 * while it lived inline — and two of the three behaviours below were wrong.
 */

import { mongoDisplayUri } from './mongoDisplay.js'
import { redisDisplayUri } from './redisUri.js'

/**
 * The text a host can be found by.
 *
 * Datastore hosts have empty `host` and `username` columns, so before this
 * they were findable only by name or group — searching for the address you
 * actually connect to returned nothing.
 *
 * Deliberately the *display* URIs: those have their credentials masked, so a
 * stored password can never be matched against, nor confirmed by a match.
 */
export function hostSearchText(host) {
  return [
    host.name,
    host.host,
    host.username,
    host.group,
    host.mongo_uri ? mongoDisplayUri(host.mongo_uri) : '',
    host.redis_uri ? redisDisplayUri(host.redis_uri) : '',
  ]
    .filter(Boolean)
    .join(' ')
    .toLowerCase()
}

/** Hosts matching a query. An empty query matches everything. */
export function filterHosts(hosts, query) {
  const q = String(query ?? '').trim().toLowerCase()
  if (!q) return hosts ?? []
  return (hosts ?? []).filter(h => hostSearchText(h).includes(q))
}

/** Ungrouped sorts last; everything else alphabetically. */
export function compareGroupNames(a, b) {
  if (!a) return 1
  if (!b) return -1
  return a.localeCompare(b)
}

/**
 * Hosts grouped for display, as an **array**.
 *
 * An object cannot express this order. `v-for` over an object iterates
 * `Object.keys`, and integer-like keys are hoisted to the front in ascending
 * numeric order however the object was built — so groups named "2" and "10"
 * rendered as 2, 10 while the comparator had asked for 10, 2. An array
 * preserves order absolutely.
 */
export function groupHosts(hosts, customGroups = []) {
  const byName = new Map()
  for (const host of hosts ?? []) {
    const name = host.group || ''
    if (!byName.has(name)) byName.set(name, [])
    byName.get(name).push(host)
  }
  // Groups the user created but has not filled yet still need to show.
  for (const name of customGroups) {
    if (!byName.has(name)) byName.set(name, [])
  }
  return [...byName.keys()]
    .sort(compareGroupNames)
    .map(name => ({ name, hosts: byName.get(name) }))
}

/** Every group name in play, for the "move to group" menu. */
export function groupNames(hosts, customGroups = []) {
  const names = new Set((hosts ?? []).map(h => h.group || ''))
  for (const g of customGroups) names.add(g)
  return [...names].sort(compareGroupNames)
}
