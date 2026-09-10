/**
 * What a host row should show about its connection.
 *
 * Split out of the component because it is the kind of thing that goes subtly
 * wrong: the previous rule was "a tab exists for this host", which stayed
 * green after the session dropped. The store already flips `connected: false`
 * when `ssh-disconnected` arrives — the truth was there and simply unread.
 *
 * Two different facts get two different signals. `connected` is "there is a
 * live session behind one of these tabs"; `active` is "the tab you are looking
 * at is this host's". A single dot cannot say both.
 */

/**
 * @param {{tabs: Array, activeTabId: ?string, connectingHostId: ?number}} view
 * @returns {{connected: boolean, active: boolean, connecting: boolean, tabCount: number}}
 */
export function hostRowState(view, hostId) {
  const tabs = view?.tabs ?? []
  const mine = tabs.filter(t => t.hostId === hostId)

  return {
    // `connected !== false` rather than `=== true`: datastore tabs never set
    // the flag at all, and they are connected by virtue of existing.
    connected: mine.some(t => t.connected !== false),
    active: mine.some(t => t.id === view?.activeTabId),
    connecting: view?.connectingHostId === hostId,
    tabCount: mine.length,
  }
}

/** How long ago, in the shortest form that is still unambiguous. */
export function formatAge(iso, now = Date.now()) {
  if (!iso) return ''
  const then = Date.parse(String(iso).includes('T') ? iso : `${iso}Z`.replace(' ', 'T'))
  if (Number.isNaN(then)) return ''

  const secs = Math.floor((now - then) / 1000)
  if (secs < 0) return 'just now'
  if (secs < 60) return 'just now'
  const mins = Math.floor(secs / 60)
  if (mins < 60) return `${mins}m ago`
  const hours = Math.floor(mins / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  if (days < 30) return `${days}d ago`
  // Past a month the exact age stops being useful; the date is more so.
  return new Date(then).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
}
