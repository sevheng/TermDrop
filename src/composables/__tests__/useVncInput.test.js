import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { ref, defineComponent, h } from 'vue'
import { mount } from '@vue/test-utils'

const calls = []
vi.mock('../../utils/invoke.js', () => ({
  invoke: (command, args) => {
    calls.push([command, args])
    return Promise.resolve()
  },
}))

const { useVncInput, MOVE_INTERVAL_MS } = await import('../useVncInput.js')

/**
 * The composable registers an unmount hook, so it needs a real component
 * context rather than being called bare.
 */
function setup({ viewOnly = false, onReleaseKeyboard } = {}) {
  const canvas = document.createElement('canvas')
  // The canvas is 1:1 with the framebuffer and at the origin, so a client
  // coordinate maps straight through and the arithmetic stays readable.
  canvas.getBoundingClientRect = () => ({ left: 0, top: 0, width: 800, height: 600 })
  document.body.appendChild(canvas)

  let api
  const Host = defineComponent({
    setup() {
      api = useVncInput({
        sessionId: ref('vnc-1'),
        canvasRef: ref(canvas),
        size: ref({ width: 800, height: 600 }),
        viewOnly: ref(viewOnly),
        onReleaseKeyboard,
      })
      return () => h('div')
    },
  })
  const wrapper = mount(Host)
  return { api, canvas, wrapper }
}

const pointer = (over = {}) => ({
  clientX: 0,
  clientY: 0,
  buttons: 0,
  pointerId: 1,
  preventDefault() {},
  ...over,
})

const key = (over = {}) => ({
  key: 'a',
  code: 'KeyA',
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  preventDefault() {},
  stopPropagation() {},
  ...over,
})

const sent = command => calls.filter(([c]) => c === command).map(([, a]) => a)

beforeEach(() => {
  calls.length = 0
  vi.useFakeTimers()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('useVncInput pointer', () => {
  it('sends a press at the position of the press, not the previous one', () => {
    // A press takes the position with it. Without that the click lands
    // wherever the pointer was last reported and misses what the user aimed
    // at -- which on a menu means choosing the wrong item.
    const { api } = setup()
    api.handlers.onPointerMove(pointer({ clientX: 10, clientY: 10 }))
    api.handlers.onPointerDown(pointer({ clientX: 400, clientY: 300, buttons: 1 }))

    const moves = sent('vnc_pointer')
    expect(moves.at(-1)).toMatchObject({ x: 400, y: 300, buttonMask: 1 })
  })

  it('translates the DOM button order into the RFB one', () => {
    // DOM is left 1 / right 2 / middle 4; RFB is left / middle / right as bits
    // 0, 1, 2. A passthrough silently swaps the middle and right buttons.
    const { api } = setup()
    api.handlers.onPointerDown(pointer({ buttons: 2 }))
    expect(sent('vnc_pointer').at(-1).buttonMask).toBe(0b100)

    api.handlers.onPointerDown(pointer({ buttons: 4 }))
    expect(sent('vnc_pointer').at(-1).buttonMask).toBe(0b010)
  })

  it('sends the first move of a gesture with no delay at all', () => {
    // The change that matters most perceptually. The old policy deferred every
    // move to a requestAnimationFrame, so the *first* move of every gesture --
    // the one the eye is waiting on -- was 0-16.7ms late.
    const { api } = setup()
    api.handlers.onPointerMove(pointer({ clientX: 100, clientY: 100 }))
    expect(sent('vnc_pointer')).toHaveLength(1)
    expect(sent('vnc_pointer')[0]).toMatchObject({ x: 100, y: 100 })
  })

  it('rate-limits a burst but never drops its final position', () => {
    // A raw pointermove stream is ~1000 events a second. Coalescing them is
    // lossless -- an RFB PointerEvent is absolute state -- but the last one
    // always matters, and that is what the trailing timer guarantees.
    const { api } = setup()
    for (let i = 1; i <= 20; i++) {
      api.handlers.onPointerMove(pointer({ clientX: i, clientY: i }))
    }
    expect(sent('vnc_pointer'), 'leading edge only').toHaveLength(1)

    vi.advanceTimersByTime(MOVE_INTERVAL_MS)
    const moves = sent('vnc_pointer')
    expect(moves).toHaveLength(2)
    expect(moves.at(-1), 'the final position must survive').toMatchObject({ x: 20, y: 20 })
  })

  it('does not re-send a position the server already has', () => {
    // At Retina resolutions in fit mode many raw moves land on the same remote
    // pixel; re-sending one is an IPC for nothing.
    const { api } = setup()
    api.handlers.onPointerMove(pointer({ clientX: 50, clientY: 50 }))
    vi.advanceTimersByTime(MOVE_INTERVAL_MS * 2)
    api.handlers.onPointerMove(pointer({ clientX: 50, clientY: 50 }))
    vi.advanceTimersByTime(MOVE_INTERVAL_MS * 2)
    expect(sent('vnc_pointer')).toHaveLength(1)
  })

  it('never lets a trailing move land after a button event', () => {
    // The press sets the position and flushes, which cancels the pending
    // trailing send. Without that, a stale coordinate arrives after the click
    // and the remote pointer jumps back.
    const { api } = setup()
    api.handlers.onPointerMove(pointer({ clientX: 10, clientY: 10 }))
    api.handlers.onPointerMove(pointer({ clientX: 20, clientY: 20 }))
    api.handlers.onPointerDown(pointer({ clientX: 400, clientY: 300, buttons: 1 }))

    vi.advanceTimersByTime(MOVE_INTERVAL_MS * 4)
    const moves = sent('vnc_pointer')
    expect(moves.at(-1)).toMatchObject({ x: 400, y: 300, buttonMask: 1 })
  })

  it('sends nothing at all when view-only', () => {
    const { api } = setup({ viewOnly: true })
    api.handlers.onPointerDown(pointer({ buttons: 1 }))
    api.handlers.onPointerMove(pointer({ clientX: 5 }))
    api.handlers.onKeyDown(key())
    expect(calls).toHaveLength(0)
  })
})

describe('useVncInput keyboard', () => {
  it('sends a keysym down and up', () => {
    const { api } = setup()
    api.handlers.onKeyDown(key())
    expect(sent('vnc_key').at(-1)).toEqual({ sessionId: 'vnc-1', keysym: 0x61, down: true })
    api.handlers.onKeyUp(key())
    expect(sent('vnc_key').at(-1)).toEqual({ sessionId: 'vnc-1', keysym: 0x61, down: false })
  })

  it('releases everything still held when focus is lost', () => {
    // Alt-Tab away with a modifier down and its keyup goes to the other
    // window, so the remote believes it is held until the session ends.
    const { api } = setup()
    api.handlers.onKeyDown(key({ key: 'Shift', code: 'ShiftLeft' }))
    api.handlers.onKeyDown(key())
    calls.length = 0

    api.handlers.onBlur()
    const ups = sent('vnc_key')
    expect(ups.every(a => a.down === false)).toBe(true)
    expect(ups.map(a => a.keysym)).toContain(0x61)
    expect(ups.map(a => a.keysym)).toContain(0xffe1)
  })

  it('hands the keyboard back on the escape combination', () => {
    // The canvas swallows every shortcut so the remote gets them, which would
    // otherwise leave no keyboard route out of the tab.
    const released = vi.fn()
    const { api } = setup({ onReleaseKeyboard: released })
    api.handlers.onKeyDown(key({ ctrlKey: true, altKey: true, shiftKey: true, code: 'ShiftLeft' }))
    expect(released).toHaveBeenCalled()
  })

  it('stops the event reaching the window shortcut handler', () => {
    // MainWindow listens on `window` with no capture flag, so it is last in
    // the bubble path -- stopPropagation is what keeps Ctrl+W from closing
    // the tab instead of reaching the remote.
    const { api } = setup()
    const stopPropagation = vi.fn()
    const preventDefault = vi.fn()
    api.handlers.onKeyDown(key({ stopPropagation, preventDefault }))
    expect(stopPropagation).toHaveBeenCalled()
    expect(preventDefault).toHaveBeenCalled()
  })
})
