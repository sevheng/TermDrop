/**
 * Geometry for the VNC canvas: how big to draw it, and where a click landed.
 *
 * Kept out of the component because every one of these is a pure function of
 * numbers, and because the device-pixel-ratio rule below is the kind of thing
 * that is much easier to assert than to eyeball on one machine.
 */

/** Fit the whole framebuffer into the viewport, letterboxed. */
export const FIT = 'fit'
/** One remote pixel per device pixel; the viewport scrolls. */
export const ACTUAL = 'actual'

/**
 * The CSS size to give the canvas element.
 *
 * The backing store is always the framebuffer's exact size and is never scaled
 * — `putImageData` ignores the transform matrix, so a scaled backing store
 * would silently misplace every raw rectangle while leaving JPEG and CopyRect
 * ones correct.
 *
 * `dpr` divides rather than multiplies, which is the opposite of the usual
 * canvas advice and is deliberate: the framebuffer's resolution is fixed by the
 * server, not by the display. Sizing the backing store to `css * dpr` — the
 * reflex — would ask the server for more pixels than exist and produce a blurry
 * desktop on every Retina screen. Dividing instead maps one remote pixel onto
 * one *device* pixel, which is as sharp as the source allows.
 */
export function vncLayout({ fbW, fbH, viewportW, viewportH, mode = FIT, dpr = 1 }) {
  const safeDpr = dpr > 0 ? dpr : 1
  if (!fbW || !fbH) return { cssW: 0, cssH: 0, scale: 1, letterboxed: false }

  if (mode === ACTUAL) {
    const cssW = fbW / safeDpr
    return { cssW, cssH: fbH / safeDpr, scale: 1 / safeDpr, letterboxed: false }
  }

  // Contain, and never enlarge: blowing a 640x480 desktop up to fill a 4K
  // window looks like a fault rather than a feature.
  const scale = Math.min(viewportW / fbW, viewportH / fbH, 1)
  return {
    cssW: fbW * scale,
    cssH: fbH * scale,
    scale,
    letterboxed: fbW * scale < viewportW || fbH * scale < viewportH,
  }
}

/**
 * Map a pointer event's client coordinates to a framebuffer pixel.
 *
 * Takes the element's live `getBoundingClientRect()` rather than a stored
 * scale, so it stays correct under any fit mode, a browser zoom, or a CSS
 * transform applied somewhere up the tree — none of which the component would
 * otherwise know had happened.
 */
export function pointerToFramebuffer(rect, fbW, fbH, clientX, clientY) {
  if (!rect?.width || !rect?.height) return { x: 0, y: 0 }
  const x = Math.round(((clientX - rect.left) * fbW) / rect.width)
  const y = Math.round(((clientY - rect.top) * fbH) / rect.height)
  return {
    x: clamp(x, 0, Math.max(0, fbW - 1)),
    y: clamp(y, 0, Math.max(0, fbH - 1)),
  }
}

function clamp(value, low, high) {
  return value < low ? low : value > high ? high : value
}

/** Pixels of scroll that make up one wheel click on the remote. */
const TICK_PX = 50
/** `deltaMode: 1` counts lines; this is the assumed line height. */
const LINE_PX = 16

/**
 * Accumulate wheel deltas into whole clicks.
 *
 * RFB has no scroll event — a click is a press and release of button 4 or 5 —
 * so a continuous delta has to be quantised. A trackpad emits dozens of
 * sub-pixel deltas per gesture, and sending a button pair for each would both
 * flood the connection and scroll the remote absurdly far.
 *
 * Returns the whole ticks to send and the remainder to carry into the next
 * call, so a slow gesture still accumulates into a click rather than being
 * rounded away each time.
 */
export function wheelTicks(accum, delta, deltaMode = 0, viewportPx = 800) {
  const px = deltaMode === 1 ? delta * LINE_PX : deltaMode === 2 ? delta * viewportPx : delta
  const total = accum + px
  const ticks = Math.trunc(total / TICK_PX)
  return { ticks, remainder: total - ticks * TICK_PX }
}

/**
 * Translate a `PointerEvent.buttons` bitmask into an RFB button mask.
 *
 * The two disagree about the middle and right buttons — DOM is
 * left/right/middle as bits 0/1/2, RFB is left/middle/right — so a passthrough
 * silently swaps them. Every web VNC client gets this wrong at least once.
 */
export function rfbButtonMask(domButtons) {
  let mask = 0
  if (domButtons & 1) mask |= 1 << 0 // left
  if (domButtons & 4) mask |= 1 << 1 // middle  (DOM bit 2)
  if (domButtons & 2) mask |= 1 << 2 // right   (DOM bit 1)
  return mask
}
