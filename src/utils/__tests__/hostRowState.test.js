import { describe, it, expect } from 'vitest'
import { hostRowState, formatAge } from '../hostRowState.js'

const view = (tabs, activeTabId = null, connectingHostId = null) => ({ tabs, activeTabId, connectingHostId })

describe('hostRowState', () => {
  it('is not connected with no tabs', () => {
    expect(hostRowState(view([]), 1)).toMatchObject({ connected: false, active: false, tabCount: 0 })
  })

  it('stops calling a dropped session connected', () => {
    // The old rule was "a tab exists", so a session that dropped still showed
    // a green dot. The store already knows better.
    const s = hostRowState(view([{ id: 'a', hostId: 1, connected: false }]), 1)
    expect(s.connected).toBe(false)
    expect(s.tabCount).toBe(1)
  })

  it('counts a datastore tab as connected even though it sets no flag', () => {
    expect(hostRowState(view([{ id: 'redis-1', hostId: 1, type: 'redis' }]), 1).connected).toBe(true)
  })

  it('is connected when any one of several tabs is live', () => {
    const s = hostRowState(view([
      { id: 'a', hostId: 1, connected: false },
      { id: 'b', hostId: 1, connected: true },
    ]), 1)
    expect(s.connected).toBe(true)
    expect(s.tabCount).toBe(2)
  })

  it('separates active from connected', () => {
    // Two facts, two signals: a host can be connected without being the tab
    // you are looking at.
    const tabs = [{ id: 'a', hostId: 1, connected: true }, { id: 'b', hostId: 2, connected: true }]
    expect(hostRowState(view(tabs, 'a'), 1)).toMatchObject({ connected: true, active: true })
    expect(hostRowState(view(tabs, 'a'), 2)).toMatchObject({ connected: true, active: false })
  })

  it('reports connecting from the store', () => {
    expect(hostRowState(view([], null, 7), 7).connecting).toBe(true)
    expect(hostRowState(view([], null, 7), 8).connecting).toBe(false)
  })

  it('does not throw on a missing view', () => {
    expect(() => hostRowState(undefined, 1)).not.toThrow()
    expect(hostRowState(undefined, 1).connected).toBe(false)
  })
})

describe('formatAge', () => {
  const now = Date.parse('2026-09-10T12:00:00Z')

  it('is empty when a host has never been connected', () => {
    expect(formatAge(null, now)).toBe('')
    expect(formatAge('', now)).toBe('')
  })

  it('reads in the shortest unambiguous form', () => {
    expect(formatAge('2026-09-10T11:59:30Z', now)).toBe('just now')
    expect(formatAge('2026-09-10T11:30:00Z', now)).toBe('30m ago')
    expect(formatAge('2026-09-10T09:00:00Z', now)).toBe('3h ago')
    expect(formatAge('2026-09-05T12:00:00Z', now)).toBe('5d ago')
  })

  it('switches to a date once the age stops being useful', () => {
    expect(formatAge('2026-01-04T12:00:00Z', now)).toMatch(/Jan/)
  })

  it('reads the space-separated form SQLite writes', () => {
    // db.rs stores CURRENT_TIMESTAMP, which has no T and no zone.
    expect(formatAge('2026-09-10 09:00:00', now)).toBe('3h ago')
  })

  it('does not throw on nonsense', () => {
    expect(formatAge('not a date', now)).toBe('')
  })

  it('treats a clock skewed into the future as now', () => {
    expect(formatAge('2026-09-10T12:05:00Z', now)).toBe('just now')
  })
})
