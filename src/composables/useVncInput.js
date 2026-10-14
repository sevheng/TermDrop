/**
 * Pointer, keyboard and clipboard for a VNC canvas.
 *
 * Split out of VncPanel because none of it is about rendering, and because
 * three of the behaviours here are the ones that make a remote desktop feel
 * broken when they are missing:
 *
 *   - a drag that leaves the canvas must keep dragging (pointer capture),
 *   - a modifier held while the window loses focus must not stay held forever,
 *   - a pointer move stream must not be forwarded event-for-event.
 */

import { ref, onBeforeUnmount } from 'vue'
import { invoke } from '../utils/invoke.js'
import { keysymFromEvent, isPhantomControlForAltGr, MODIFIER_KEYSYMS } from '../utils/keysym.js'
import { pointerToFramebuffer, wheelTicks, rfbButtonMask } from '../utils/vncLayout.js'

/**
 * The shortest gap between two pointer sends, in milliseconds.
 *
 * 125Hz: above the rate at which more reports help, below the rate at which
 * the IPC volume matters, and comfortably inside one display frame so the
 * trailing send is never perceptible.
 */
export const MOVE_INTERVAL_MS = 8

/** RFB wheel buttons: up, down, left, right. */
const WHEEL_UP = 1 << 3
const WHEEL_DOWN = 1 << 4
const WHEEL_LEFT = 1 << 5
const WHEEL_RIGHT = 1 << 6

export function useVncInput({ sessionId, canvasRef, size, viewOnly, onReleaseKeyboard }) {
  /** Keysyms currently held down, so they can all be released on blur. */
  const downKeys = new Set()
  /** When ControlLeft went down, for the AltGr heuristic. */
  let controlDownAt = null
  /**
   * The latest pointer position, as raw client coordinates.
   *
   * Deliberately *not* mapped to framebuffer pixels yet: mapping needs the
   * canvas's bounding rect, and doing it per event meant a layout read on all
   * ~1000 raw moves a second. Deferring it to the send cuts that to 125, and
   * is more correct than caching the rect would be -- the rect is always the
   * live one.
   */
  let pendingClient = null
  let trailingTimer = null
  // Not 0: `performance.now()` starts near zero itself, so the first moves
  // after a page load would look like they had just been sent and take the
  // trailing path instead of going out immediately.
  let lastSentAt = -Infinity
  let lastSent = null
  let buttonMask = 0
  let wheelX = 0
  let wheelY = 0

  const hasFocus = ref(false)

  function send(command, args) {
    if (!sessionId.value) return
    invoke(command, { sessionId: sessionId.value, ...args }).catch(() => {})
  }

  // -- pointer --------------------------------------------------------------

  function positionOf(event) {
    return mapClient(event.clientX, event.clientY)
  }

  function mapClient(clientX, clientY) {
    const canvas = canvasRef.value
    if (!canvas) return null
    // The live rect, not a stored scale: this then stays correct under any fit
    // mode, a browser zoom, or a transform applied further up the tree.
    return pointerToFramebuffer(
      canvas.getBoundingClientRect(),
      size.value.width,
      size.value.height,
      clientX,
      clientY,
    )
  }

  /**
   * Send the pending position now, and stop any trailing send.
   *
   * An RFB PointerEvent is absolute *state*, not a delta, so dropping
   * intermediate moves loses nothing — but the *final* one always matters, and
   * that is what the trailing timer guarantees.
   */
  function flushMove() {
    clearTimeout(trailingTimer)
    trailingTimer = null
    if (!pendingClient) return

    const at = mapClient(pendingClient.x, pendingClient.y)
    pendingClient = null
    if (!at) return

    // At Retina resolutions in fit mode many raw moves land on the same remote
    // pixel; re-sending one the server already has costs an IPC for nothing.
    if (lastSent && lastSent.x === at.x && lastSent.y === at.y && lastSent.mask === buttonMask) {
      return
    }
    lastSent = { x: at.x, y: at.y, mask: buttonMask }
    lastSentAt = performance.now()
    send('vnc_pointer', { x: at.x, y: at.y, buttonMask })
  }

  /**
   * Leading edge, then a trailing flush.
   *
   * The old policy deferred every move to a `requestAnimationFrame`, which put
   * 0-16.7ms in front of the *first* move of every gesture — the one the eye
   * is actually waiting on. After any idle the send now goes out immediately;
   * only a continuous drag is rate-limited.
   */
  function onPointerMove(event) {
    if (viewOnly.value) return
    pendingClient = { x: event.clientX, y: event.clientY }

    const since = performance.now() - lastSentAt
    if (since >= MOVE_INTERVAL_MS) {
      flushMove()
    } else if (trailingTimer === null) {
      trailingTimer = setTimeout(flushMove, MOVE_INTERVAL_MS - since)
    }
  }

  function onPointerDown(event) {
    canvasRef.value?.focus()
    if (viewOnly.value) return
    // A button press is never rate-limited: it is the event with the least
    // tolerance for delay, and there is one of it.
    // Without capture, dragging a window past the edge of the canvas drops the
    // rest of the gesture and the remote is left with the button still held.
    canvasRef.value?.setPointerCapture?.(event.pointerId)
    applyButtons(event)
  }

  function onPointerUp(event) {
    if (viewOnly.value) return
    canvasRef.value?.releasePointerCapture?.(event.pointerId)
    applyButtons(event)
  }

  function applyButtons(event) {
    // The press position wins over whatever move was pending, or the click
    // lands where the pointer used to be and misses what the user aimed at.
    // `flushMove` clears the trailing timer, so no stale position can follow.
    pendingClient = { x: event.clientX, y: event.clientY }
    buttonMask = rfbButtonMask(event.buttons)
    lastSent = null
    flushMove()
  }

  function onWheel(event) {
    if (viewOnly.value) return
    event.preventDefault()
    const at = positionOf(event)
    if (!at) return

    const vertical = wheelTicks(wheelY, event.deltaY, event.deltaMode, window.innerHeight)
    const horizontal = wheelTicks(wheelX, event.deltaX, event.deltaMode, window.innerWidth)
    wheelY = vertical.remainder
    wheelX = horizontal.remainder

    emitTicks(at, vertical.ticks, WHEEL_DOWN, WHEEL_UP)
    emitTicks(at, horizontal.ticks, WHEEL_RIGHT, WHEEL_LEFT)
  }

  /** RFB has no scroll event: a click is a press and release of a button. */
  function emitTicks(at, ticks, positiveButton, negativeButton) {
    const button = ticks > 0 ? positiveButton : negativeButton
    for (let i = 0; i < Math.abs(ticks); i++) {
      send('vnc_pointer', { ...at, buttonMask: buttonMask | button })
      send('vnc_pointer', { ...at, buttonMask })
    }
  }

  // -- keyboard -------------------------------------------------------------

  function onKeyDown(event) {
    // Everything, including Ctrl+W and Tab: a VNC viewer that lets the browser
    // keep its own shortcuts cannot be used for anything real. The escape
    // hatch below is what keeps that from trapping the user.
    event.preventDefault()
    event.stopPropagation()

    // Ctrl+Alt+Shift releases the keyboard and hands focus back to the app.
    // Without one deliberate way out, there is no keyboard route out of the
    // tab at all.
    if (event.ctrlKey && event.altKey && event.shiftKey) {
      releaseAllKeys()
      onReleaseKeyboard?.()
      return
    }

    if (viewOnly.value) return

    const { keysym, ok } = keysymFromEvent(event)
    if (!ok) return

    if (event.code === 'ControlLeft') controlDownAt = performance.now()
    if (event.code === 'AltRight' && isPhantomControlForAltGr(controlDownAt, performance.now())) {
      // That Control was AltGr, not a Control. It has already gone out, so
      // take it back rather than trying to suppress it.
      releaseKey(0xffe3)
      controlDownAt = null
    }

    downKeys.add(keysym)
    send('vnc_key', { keysym, down: true })
  }

  function onKeyUp(event) {
    event.preventDefault()
    event.stopPropagation()
    if (viewOnly.value) return

    const { keysym, ok } = keysymFromEvent(event)
    if (!ok) return
    if (event.code === 'ControlLeft') controlDownAt = null
    releaseKey(keysym)
  }

  function releaseKey(keysym) {
    if (!downKeys.delete(keysym)) return
    send('vnc_key', { keysym, down: false })
  }

  /**
   * Release everything still held.
   *
   * The bug this exists for: Alt-Tab away with Alt down and the keyup is
   * delivered to the other window, so the remote believes Alt is held until
   * the session ends. Every modifier is released unconditionally, because a
   * stuck one we never saw go down is exactly the case that matters.
   */
  function releaseAllKeys() {
    for (const keysym of [...downKeys]) releaseKey(keysym)
    for (const keysym of MODIFIER_KEYSYMS) send('vnc_key', { keysym, down: false })
    downKeys.clear()
    controlDownAt = null
  }

  function onFocus() {
    hasFocus.value = true
  }

  function onBlur() {
    hasFocus.value = false
    releaseAllKeys()
  }

  function onVisibilityChange() {
    if (document.visibilityState === 'hidden') releaseAllKeys()
  }

  function onContextMenu(event) {
    // Otherwise a right-click opens the app's menu instead of the remote's.
    event.preventDefault()
  }

  document.addEventListener('visibilitychange', onVisibilityChange)
  onBeforeUnmount(() => {
    document.removeEventListener('visibilitychange', onVisibilityChange)
    clearTimeout(trailingTimer)
  })

  return {
    hasFocus,
    releaseAllKeys,
    handlers: {
      onPointerMove,
      onPointerDown,
      onPointerUp,
      onPointerCancel: onPointerUp,
      onWheel,
      onKeyDown,
      onKeyUp,
      onFocus,
      onBlur,
      onContextMenu,
    },
  }
}
