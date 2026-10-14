import { describe, it, expect } from 'vitest'
import { parseFrame, fillColour, MAGIC, VERSION, OP } from '../vncFrame.js'

/** Build a frame the way the Rust FrameBuilder does. */
function frame(seq, rects) {
  const payload = rects.reduce((n, r) => n + 16 + r.payload.length, 0)
  const buf = new ArrayBuffer(16 + payload)
  const view = new DataView(buf)
  const bytes = new Uint8Array(buf)
  view.setUint32(0, MAGIC, true)
  view.setUint16(4, VERSION, true)
  view.setUint16(6, rects.length, true)
  view.setUint32(8, seq, true)
  view.setUint32(12, payload, true)

  let at = 16
  for (const r of rects) {
    view.setUint16(at, r.x ?? 0, true)
    view.setUint16(at + 2, r.y ?? 0, true)
    view.setUint16(at + 4, r.width ?? 0, true)
    view.setUint16(at + 6, r.height ?? 0, true)
    view.setUint16(at + 8, r.op, true)
    view.setUint16(at + 10, 0, true)
    view.setUint32(at + 12, r.payload.length, true)
    bytes.set(r.payload, at + 16)
    at += 16 + r.payload.length
  }
  return buf
}

describe('parseFrame', () => {
  it('reads the header and every rectangle in wire order', () => {
    // Order is apply order: a CopyRect that overtook the JPEG it depends on
    // would paint stale pixels.
    const parsed = parseFrame(
      frame(42, [
        { op: OP.JPEG, width: 8, height: 8, payload: new Uint8Array([0xff, 0xd8]) },
        { op: OP.COPY_RECT, x: 8, payload: new Uint8Array([1, 0, 2, 0]) },
        { op: OP.FILL, width: 4, height: 4, payload: new Uint8Array([9, 8, 7, 255]) },
      ]),
    )
    expect(parsed.seq).toBe(42)
    expect(parsed.rects.map(r => r.op)).toEqual([OP.JPEG, OP.COPY_RECT, OP.FILL])
    expect(parsed.rects[1].x).toBe(8)
  })

  it('returns payloads as views, not copies', () => {
    // A full-screen raw rectangle is 8MB; copying it here would double both
    // the allocation and the time before it reaches the canvas.
    const buf = frame(0, [{ op: OP.RAW_RGBA, width: 1, height: 1, payload: new Uint8Array([1, 2, 3, 4]) }])
    const { rects } = parseFrame(buf)
    expect(rects[0].payload.buffer).toBe(buf)
    expect(Array.from(rects[0].payload)).toEqual([1, 2, 3, 4])
  })

  it('accepts a frame with no rectangles', () => {
    expect(parseFrame(frame(7, [])).rects).toEqual([])
  })

  it('rejects a frame that is not one of ours', () => {
    // The magic turns a desync into an error on the first frame rather than a
    // garbled picture on the hundredth.
    const buf = frame(0, [])
    new DataView(buf).setUint32(0, 0xdeadbeef, true)
    expect(() => parseFrame(buf)).toThrow(/magic/)
  })

  it('rejects a version it cannot parse', () => {
    const buf = frame(0, [])
    new DataView(buf).setUint16(4, 99, true)
    expect(() => parseFrame(buf)).toThrow(/version/)
  })

  it('refuses to read past the end of the buffer', () => {
    // Every length here came off a socket, so a wrong one must be an error and
    // never an out-of-range view.
    expect(() => parseFrame(new ArrayBuffer(8))).toThrow(/too short/)

    const buf = frame(0, [{ op: OP.RAW_RGBA, payload: new Uint8Array([1, 2, 3, 4]) }])
    new DataView(buf).setUint32(28, 9999, true) // the rect's length field
    expect(() => parseFrame(buf)).toThrow(/past the frame/)

    const short = frame(0, [{ op: OP.FILL, payload: new Uint8Array([1, 2, 3, 4]) }])
    new DataView(short).setUint16(6, 5, true) // claim five rectangles
    expect(() => parseFrame(short)).toThrow(/mid-rectangle/)
  })
})

describe('fillColour', () => {
  it('ignores the alpha byte, which is always opaque', () => {
    expect(fillColour(new Uint8Array([34, 68, 102, 255]))).toBe('rgb(34 68 102)')
  })
})
