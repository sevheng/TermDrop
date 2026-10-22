import { describe, it, expect, beforeEach, vi } from 'vitest'

const db = new Map()
const invokes = []
vi.mock('../../utils/invoke.js', () => ({
  invoke: (command, args) => {
    invokes.push([command, args])
    if (command === 'get_setting') return Promise.resolve(db.get(args.key) ?? null)
    if (command === 'set_setting') {
      db.set(args.key, args.value)
      return Promise.resolve(null)
    }
    return Promise.reject(new Error(`unexpected ${command}`))
  },
}))

const { usePref, savePref, loadPrefs, resetPrefsForTest } = await import('../usePrefs.js')

const sets = () => invokes.filter(([c]) => c === 'set_setting')

beforeEach(() => {
  db.clear()
  invokes.length = 0
  localStorage.clear()
  resetPrefsForTest()
})

describe('usePref', () => {
  it('starts from the localStorage cache, parsed', () => {
    localStorage.setItem('sidebar-width', '300')
    localStorage.setItem('host-collapsed-groups', '["a","b"]')
    expect(usePref('sidebar-width', 220).value).toBe(300)
    expect(usePref('host-collapsed-groups', []).value).toEqual(['a', 'b'])
  })

  it('falls back when the cache is missing or unreadable', () => {
    localStorage.setItem('sidebar-width', 'wide')
    localStorage.setItem('sftp-columns', '{not json')
    expect(usePref('sidebar-width', 220).value).toBe(220)
    expect(usePref('sftp-columns', {}).value).toEqual({})
  })

  it('hands every caller the same ref', () => {
    expect(usePref('host-view-mode', 'grouped')).toBe(usePref('host-view-mode', 'flat'))
  })

  it('refuses a key it does not know, so a typo cannot persist silently', () => {
    expect(() => usePref('sidebar-wdith', 1)).toThrow(/unknown preference/)
  })
})

describe('loadPrefs', () => {
  it('lets SQLite win over the cache and refreshes the cache', async () => {
    localStorage.setItem('sidebar-width', '300')
    db.set('sidebar-width', '180')
    const width = usePref('sidebar-width', 220)
    await loadPrefs()
    expect(width.value).toBe(180)
    expect(localStorage.getItem('sidebar-width')).toBe('180')
    expect(sets()).toEqual([])
  })

  it('migrates a value that exists only in localStorage', async () => {
    localStorage.setItem('host-custom-groups', '["empty-group"]')
    localStorage.setItem('host-view-mode', 'flat')
    await loadPrefs()
    expect(db.get('host-custom-groups')).toBe('["empty-group"]')
    expect(db.get('host-view-mode')).toBe('flat')
  })

  it('does not migrate a cached value it cannot parse', async () => {
    localStorage.setItem('sftp-width', 'NaN')
    await loadPrefs()
    expect(db.has('sftp-width')).toBe(false)
  })

  it('reaches a ref created after it ran, through the refreshed cache', async () => {
    db.set('sftp-columns', '{"size":true}')
    await loadPrefs()
    expect(usePref('sftp-columns', {}).value).toEqual({ size: true })
  })
})

describe('savePref', () => {
  it('writes SQLite and the cache', async () => {
    const mode = usePref('host-view-mode', 'grouped')
    mode.value = 'flat'
    await savePref('host-view-mode')
    expect(db.get('host-view-mode')).toBe('flat')
    expect(localStorage.getItem('host-view-mode')).toBe('flat')
  })

  it('skips the IPC when SQLite already holds the value', async () => {
    db.set('sidebar-width', '250')
    const width = usePref('sidebar-width', 220)
    await loadPrefs()
    expect(width.value).toBe(250)
    await savePref('sidebar-width')
    expect(sets()).toEqual([])
    width.value = 260
    await savePref('sidebar-width')
    await savePref('sidebar-width')
    expect(sets()).toEqual([['set_setting', { key: 'sidebar-width', value: '260' }]])
  })
})
