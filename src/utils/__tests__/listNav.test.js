import { describe, it, expect } from 'vitest'
import {
  visibleRows, nextRow, firstRow, lastRow, pageRow, navIntent, HOST_ROW_H,
} from '../listNav.js'

const groups = [
  { name: 'web', hosts: [{ id: 1 }, { id: 2 }] },
  { name: 'db', hosts: [{ id: 3 }] },
]

describe('visibleRows', () => {
  it('interleaves group headers with their hosts', () => {
    expect(visibleRows(groups).map(r => r.kind)).toEqual(['group', 'host', 'host', 'group', 'host'])
  })

  it('omits the rows of a collapsed group entirely', () => {
    // Omitted, not hidden: arrowing must skip them, and only absence says so.
    const rows = visibleRows(groups, new Set(['web']))
    expect(rows.map(r => r.kind)).toEqual(['group', 'group', 'host'])
    expect(rows.filter(r => r.kind === 'host')).toHaveLength(1)
  })

  it('survives empty and missing input', () => {
    expect(visibleRows([])).toEqual([])
    expect(visibleRows(undefined)).toEqual([])
    expect(visibleRows([{ name: 'x' }])).toHaveLength(1)
  })
})

describe('nextRow', () => {
  const rows = visibleRows(groups)

  it('skips group headers', () => {
    // index 0 is a header, 1 and 2 are hosts, 3 a header, 4 a host
    expect(nextRow(rows, 1, 1)).toBe(2)
    expect(nextRow(rows, 2, 1)).toBe(4)
    expect(nextRow(rows, 4, -1)).toBe(2)
  })

  it('stops at the ends rather than wrapping', () => {
    // Wrapping in a long list loses your place more often than it helps.
    expect(nextRow(rows, 4, 1)).toBe(4)
    expect(nextRow(rows, 1, -1)).toBe(1)
  })

  it('finds the first and last selectable rows', () => {
    expect(firstRow(rows)).toBe(1)
    expect(lastRow(rows)).toBe(4)
  })

  it('returns -1 when nothing is selectable', () => {
    const onlyHeaders = visibleRows([{ name: 'a' }, { name: 'b' }])
    expect(firstRow(onlyHeaders)).toBe(-1)
    expect(nextRow([], 0, 1)).toBe(-1)
  })
})

describe('pageRow', () => {
  const many = visibleRows([{ name: 'g', hosts: Array.from({ length: 30 }, (_, i) => ({ id: i })) }])

  it('moves a page and stops at the end', () => {
    expect(pageRow(many, 1, 1, 10)).toBe(11)
    expect(pageRow(many, 25, 1, 10)).toBe(30)
    expect(pageRow(many, 5, -1, 10)).toBe(1)
  })
})

describe('navIntent', () => {
  it('maps the movement keys', () => {
    expect(navIntent('ArrowDown')).toEqual({ type: 'move', dir: 1 })
    expect(navIntent('ArrowUp')).toEqual({ type: 'move', dir: -1 })
    expect(navIntent('Home')).toEqual({ type: 'first' })
    expect(navIntent('End')).toEqual({ type: 'last' })
    expect(navIntent('Enter')).toEqual({ type: 'activate' })
  })

  it('treats the arrows as tree controls only on a group', () => {
    expect(navIntent('ArrowRight', { onGroup: true, collapsed: true })).toEqual({ type: 'expand' })
    expect(navIntent('ArrowLeft', { onGroup: true, collapsed: false })).toEqual({ type: 'collapse' })
    // Already expanded: nothing to open.
    expect(navIntent('ArrowRight', { onGroup: true, collapsed: false })).toBeNull()
  })

  it('leaves right-arrow alone on a host, so a text cursor still works', () => {
    expect(navIntent('ArrowRight', { onGroup: false })).toBeNull()
  })

  it('claims nothing it does not own', () => {
    for (const k of ['a', 'Tab', 'Shift', 'F5', ' ']) expect(navIntent(k)).toBeNull()
  })
})

describe('HOST_ROW_H', () => {
  it('matches the h-8 the row class declares', () => {
    // One constant, so a virtual list and the CSS cannot disagree.
    expect(HOST_ROW_H).toBe(32)
  })
})
