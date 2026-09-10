import { describe, it, expect } from 'vitest'
import { nextFocusIndex, visibleOnly, FOCUSABLE } from '../focusTrap.js'

describe('nextFocusIndex', () => {
  it('wraps, because a dialog is a closed loop', () => {
    // Unlike a list, falling off the end has nowhere to go but the
    // application behind the scrim.
    expect(nextFocusIndex(3, 2, 1)).toBe(0)
    expect(nextFocusIndex(3, 0, -1)).toBe(2)
  })

  it('steps normally in the middle', () => {
    expect(nextFocusIndex(3, 0, 1)).toBe(1)
    expect(nextFocusIndex(3, 2, -1)).toBe(1)
  })

  it('enters from outside at the right end', () => {
    expect(nextFocusIndex(3, -1, 1)).toBe(0)
    expect(nextFocusIndex(3, -1, -1)).toBe(2)
  })

  it('reports nothing to focus for an empty dialog', () => {
    expect(nextFocusIndex(0, -1, 1)).toBe(-1)
  })
})

describe('visibleOnly', () => {
  it('drops elements with no boxes', () => {
    const shown = { offsetWidth: 40, offsetHeight: 20, getClientRects: () => [{}] }
    const hidden = { offsetWidth: 0, offsetHeight: 0, getClientRects: () => [] }
    expect(visibleOnly([shown, hidden])).toEqual([shown])
  })

  it('fails open when no element reports a box', () => {
    // That means layout is unavailable, not that everything is hidden.
    // Filtering here would empty the list and disable the trap entirely,
    // letting Tab escape the dialog — the exact bug the trap prevents.
    const a = { offsetWidth: 0, offsetHeight: 0, getClientRects: () => [] }
    const b = { offsetWidth: 0, offsetHeight: 0, getClientRects: () => [] }
    expect(visibleOnly([a, b])).toEqual([a, b])
  })

  it('handles a missing list', () => {
    expect(visibleOnly(undefined)).toEqual([])
  })
})

describe('FOCUSABLE', () => {
  it('excludes disabled controls and tabindex -1', () => {
    expect(FOCUSABLE).toContain(':not([disabled])')
    expect(FOCUSABLE).toContain('[tabindex]:not([tabindex="-1"])')
  })
})
