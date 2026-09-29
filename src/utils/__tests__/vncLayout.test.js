import { describe, it, expect } from 'vitest'
import {
  vncLayout,
  pointerToFramebuffer,
  wheelTicks,
  rfbButtonMask,
  FIT,
  ACTUAL,
} from '../vncLayout.js'

describe('vncLayout', () => {
  it('contains the framebuffer in the viewport', () => {
    const l = vncLayout({ fbW: 1920, fbH: 1080, viewportW: 960, viewportH: 1000, mode: FIT })
    expect(l.scale).toBeCloseTo(0.5)
    expect(l.cssW).toBeCloseTo(960)
    expect(l.cssH).toBeCloseTo(540)
    expect(l.letterboxed).toBe(true)
  })

  it('never enlarges past 1:1', () => {
    // A 640x480 desktop blown up to fill a 4K window looks like a fault.
    const l = vncLayout({ fbW: 640, fbH: 480, viewportW: 3840, viewportH: 2160, mode: FIT })
    expect(l.scale).toBe(1)
    expect(l.cssW).toBe(640)
  })

  it('divides by the device pixel ratio at actual size, rather than multiplying', () => {
    // The reflex is to multiply, which asks the server for pixels that do not
    // exist and gives a blurry desktop on every Retina screen. The framebuffer
    // resolution is fixed by the server, so one remote pixel is mapped onto one
    // *device* pixel instead.
    const l = vncLayout({ fbW: 1920, fbH: 1080, viewportW: 800, viewportH: 600, mode: ACTUAL, dpr: 2 })
    expect(l.cssW).toBe(960)
    expect(l.cssH).toBe(540)
  })

  it('is 1:1 in CSS pixels on a non-retina display', () => {
    const l = vncLayout({ fbW: 1024, fbH: 768, viewportW: 500, viewportH: 500, mode: ACTUAL, dpr: 1 })
    expect(l.cssW).toBe(1024)
    expect(l.cssH).toBe(768)
  })

  it('survives a zero framebuffer or a nonsense ratio', () => {
    expect(vncLayout({ fbW: 0, fbH: 0, viewportW: 100, viewportH: 100 }).cssW).toBe(0)
    const l = vncLayout({ fbW: 100, fbH: 100, viewportW: 50, viewportH: 50, mode: ACTUAL, dpr: 0 })
    expect(Number.isFinite(l.cssW)).toBe(true)
  })
})

describe('pointerToFramebuffer', () => {
  const rect = { left: 10, top: 20, width: 960, height: 540 }

  it('scales a click through whatever the element is currently sized to', () => {
    // Using the live rect rather than a stored scale keeps this right under
    // any fit mode, a browser zoom, or a transform further up the tree.
    expect(pointerToFramebuffer(rect, 1920, 1080, 10, 20)).toEqual({ x: 0, y: 0 })
    expect(pointerToFramebuffer(rect, 1920, 1080, 490, 290)).toEqual({ x: 960, y: 540 })
  })

  it('clamps to the last real pixel rather than one past it', () => {
    // A click on the very edge would otherwise address column 1920 of a
    // 1920-wide framebuffer.
    expect(pointerToFramebuffer(rect, 1920, 1080, 970, 560)).toEqual({ x: 1919, y: 1079 })
    expect(pointerToFramebuffer(rect, 1920, 1080, -100, -100)).toEqual({ x: 0, y: 0 })
  })

  it('does not divide by a zero-sized element', () => {
    expect(pointerToFramebuffer({ left: 0, top: 0, width: 0, height: 0 }, 100, 100, 5, 5))
      .toEqual({ x: 0, y: 0 })
    expect(pointerToFramebuffer(null, 100, 100, 5, 5)).toEqual({ x: 0, y: 0 })
  })
})

describe('wheelTicks', () => {
  it('carries a remainder so a slow gesture still scrolls', () => {
    // A trackpad emits dozens of sub-pixel deltas; rounding each to zero would
    // make it scroll nothing at all.
    let { ticks, remainder } = wheelTicks(0, 20)
    expect(ticks).toBe(0)
    ;({ ticks, remainder } = wheelTicks(remainder, 20))
    expect(ticks).toBe(0)
    ;({ ticks, remainder } = wheelTicks(remainder, 20))
    expect(ticks).toBe(1)
    expect(remainder).toBeCloseTo(10)
  })

  it('emits several clicks for one large jump', () => {
    expect(wheelTicks(0, 160).ticks).toBe(3)
  })

  it('scrolls both ways', () => {
    expect(wheelTicks(0, -120).ticks).toBe(-2)
  })

  it('converts line and page deltas to pixels', () => {
    // Firefox reports lines, not pixels; untranslated, one notch would be
    // three pixels and the remote would never scroll.
    expect(wheelTicks(0, 3, 1).ticks).toBe(0)
    expect(wheelTicks(0, 12, 1).ticks).toBe(3)
    expect(wheelTicks(0, 1, 2, 800).ticks).toBe(16)
  })
})

describe('rfbButtonMask', () => {
  it('swaps middle and right, which the two conventions disagree about', () => {
    // DOM: left 1, right 2, middle 4. RFB: left, middle, right as bits 0,1,2.
    // A passthrough silently transposes them, which is the classic bug here.
    expect(rfbButtonMask(1)).toBe(0b001)
    expect(rfbButtonMask(4)).toBe(0b010)
    expect(rfbButtonMask(2)).toBe(0b100)
  })

  it('carries several buttons held at once', () => {
    expect(rfbButtonMask(1 | 2)).toBe(0b101)
    expect(rfbButtonMask(0)).toBe(0)
  })
})
