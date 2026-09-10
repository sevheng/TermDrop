import { describe, it, expect } from 'vitest'
import {
  nextEnabledIndex,
  firstEnabledIndex,
  lastEnabledIndex,
  indexOfValue,
  typeaheadIndex,
  menuPlacement,
  MAX_MENU_HEIGHT,
} from '../selectNav.js'

const opts = [
  { value: '', label: 'All types' },
  { value: 'string', label: 'String' },
  { value: 'stream', label: 'Stream' },
  { value: 'hash', label: 'Hash' },
]

describe('nextEnabledIndex', () => {
  it('steps and wraps in both directions', () => {
    expect(nextEnabledIndex(opts, 0, 1)).toBe(1)
    expect(nextEnabledIndex(opts, 3, 1)).toBe(0)
    expect(nextEnabledIndex(opts, 0, -1)).toBe(3)
    expect(nextEnabledIndex(opts, 2, -1)).toBe(1)
  })

  it('skips disabled options', () => {
    const o = [{ label: 'a' }, { label: 'b', disabled: true }, { label: 'c' }]
    expect(nextEnabledIndex(o, 0, 1)).toBe(2)
    expect(nextEnabledIndex(o, 2, -1)).toBe(0)
  })

  it('gives up rather than spinning when nothing is selectable', () => {
    const o = [{ label: 'a', disabled: true }, { label: 'b', disabled: true }]
    expect(nextEnabledIndex(o, 0, 1)).toBe(-1)
    expect(nextEnabledIndex([], 0, 1)).toBe(-1)
  })
})

describe('first/lastEnabledIndex', () => {
  it('finds the ends', () => {
    expect(firstEnabledIndex(opts)).toBe(0)
    expect(lastEnabledIndex(opts)).toBe(3)
  })

  it('respects disabled at the ends', () => {
    const o = [{ label: 'a', disabled: true }, { label: 'b' }, { label: 'c', disabled: true }]
    expect(firstEnabledIndex(o)).toBe(1)
    expect(lastEnabledIndex(o)).toBe(1)
  })
})

describe('indexOfValue', () => {
  it('finds a value including the empty one', () => {
    // The "no filter" option has value '' and must not be confused with
    // "not found" -- a === comparison, never a falsy test.
    expect(indexOfValue(opts, '')).toBe(0)
    expect(indexOfValue(opts, 'hash')).toBe(3)
    expect(indexOfValue(opts, 'nope')).toBe(-1)
  })

  it('matches numeric values by identity', () => {
    const dbs = [{ value: 0, label: 'db0' }, { value: 1, label: 'db1' }]
    expect(indexOfValue(dbs, 0)).toBe(0)
    expect(indexOfValue(dbs, 1)).toBe(1)
  })
})

describe('typeaheadIndex', () => {
  it('matches on a label prefix, case-insensitively', () => {
    expect(typeaheadIndex(opts, 'h', -1)).toBe(3)
    expect(typeaheadIndex(opts, 'HASH', -1)).toBe(3)
  })

  it('walks through options sharing a prefix instead of sticking', () => {
    // "String" then "Stream": pressing s twice must move on.
    expect(typeaheadIndex(opts, 's', 0)).toBe(1)
    expect(typeaheadIndex(opts, 's', 1)).toBe(2)
    expect(typeaheadIndex(opts, 's', 2)).toBe(1)
  })

  it('narrows as more letters arrive', () => {
    expect(typeaheadIndex(opts, 'stre', -1)).toBe(2)
  })

  it('returns -1 for no match or an empty query', () => {
    expect(typeaheadIndex(opts, 'zzz', -1)).toBe(-1)
    expect(typeaheadIndex(opts, '', -1)).toBe(-1)
  })
})

describe('menuPlacement', () => {
  const anchor = (top, bottom) => ({ top, bottom })

  it('opens below when there is room', () => {
    const p = menuPlacement(anchor(100, 130), 200, 800)
    expect(p.placeAbove).toBe(false)
    expect(p.top).toBe(134)
  })

  it('flips above when below is too tight and above is roomier', () => {
    const p = menuPlacement(anchor(600, 630), 200, 700)
    expect(p.placeAbove).toBe(true)
    // Sits its own height above the trigger, not below the fold.
    expect(p.top).toBeLessThan(600)
  })

  it('stays below when neither side fits but below is roomier', () => {
    const p = menuPlacement(anchor(40, 70), 900, 500)
    expect(p.placeAbove).toBe(false)
    expect(p.maxHeight).toBeLessThan(900)
  })

  it('caps the height so a long list scrolls instead of overflowing', () => {
    const p = menuPlacement(anchor(100, 130), 5000, 800)
    expect(p.maxHeight).toBeLessThanOrEqual(800 - 130)
    expect(p.maxHeight).toBeGreaterThan(0)
  })

  it('never fills the window, however much room there is', () => {
    // A 20-host list in a tall window used to open ~550px tall and bury the
    // dialog it belonged to.
    const p = menuPlacement(anchor(560, 590), 5000, 900)
    expect(p.maxHeight).toBeLessThanOrEqual(MAX_MENU_HEIGHT)
  })

  it('still flips above using the capped height, not the raw one', () => {
    // 5000px does not fit below, but the capped 288 does — so it should stay
    // below rather than flipping for a height it will never use.
    const p = menuPlacement(anchor(100, 130), 5000, 800)
    expect(p.placeAbove).toBe(false)
  })

  it('never returns a uselessly small menu', () => {
    const p = menuPlacement(anchor(395, 400), 300, 410)
    expect(p.maxHeight).toBeGreaterThanOrEqual(96)
  })
})
