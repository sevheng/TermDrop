/**
 * Reading the binary frames the VNC backend sends.
 *
 * One message is one complete framebuffer update, so a frame is either applied
 * whole or not at all and the canvas never shows a half-painted picture.
 *
 * Little-endian throughout, which is the *opposite* of RFB on the wire — the
 * backend converts once so this side never has to byte-swap.
 */

/** Spells "TDVB". A desync shows up on the first frame instead of the hundredth. */
export const MAGIC = 0x42564454
export const VERSION = 1
const HEADER_LEN = 16
const RECT_HEADER_LEN = 16

export const OP = {
  RAW_RGBA: 1,
  COPY_RECT: 2,
  JPEG: 3,
  FILL: 4,
  DESKTOP_SIZE: 5,
  CURSOR: 6,
}

/**
 * Parse a frame into an ordered list of operations.
 *
 * Returns `{ seq, rects }`, or throws. Payloads are returned as **views onto
 * the original buffer**, not copies: a full-screen raw rectangle is 8MB and
 * copying it here would double both the allocation and the time before it
 * reaches the canvas.
 */
export function parseFrame(buffer) {
  const view = new DataView(buffer)
  if (buffer.byteLength < HEADER_LEN) {
    throw new Error(`frame too short: ${buffer.byteLength} bytes`)
  }
  const magic = view.getUint32(0, true)
  if (magic !== MAGIC) {
    throw new Error(`bad frame magic 0x${magic.toString(16)}`)
  }
  const version = view.getUint16(4, true)
  if (version !== VERSION) {
    throw new Error(`unsupported frame version ${version}`)
  }

  const count = view.getUint16(6, true)
  const seq = view.getUint32(8, true)
  const payloadLen = view.getUint32(12, true)
  if (HEADER_LEN + payloadLen > buffer.byteLength) {
    throw new Error('frame payload is shorter than its header claims')
  }

  const rects = []
  let at = HEADER_LEN
  for (let i = 0; i < count; i++) {
    if (at + RECT_HEADER_LEN > buffer.byteLength) {
      throw new Error(`frame ended mid-rectangle at ${at}`)
    }
    const rect = {
      x: view.getUint16(at, true),
      y: view.getUint16(at + 2, true),
      width: view.getUint16(at + 4, true),
      height: view.getUint16(at + 6, true),
      op: view.getUint16(at + 8, true),
      // at + 10 is padding, which keeps every payload 4-byte aligned.
    }
    const len = view.getUint32(at + 12, true)
    const start = at + RECT_HEADER_LEN
    if (start + len > buffer.byteLength) {
      throw new Error(`rectangle payload runs past the frame at ${start}`)
    }
    rect.payload = new Uint8Array(buffer, start, len)
    rects.push(rect)
    at = start + len
  }

  return { seq, rects }
}

/** The CSS colour of a FILL rectangle's four payload bytes. */
export function fillColour(payload) {
  return `rgb(${payload[0]} ${payload[1]} ${payload[2]})`
}

/**
 * FNV-1a over a byte array.
 *
 * Used to recognise a cursor bitmap we have already encoded. Not a checksum
 * and not security-relevant — a collision would show the wrong cursor shape,
 * which is why the caller also keys on the dimensions. Cheap enough to run on
 * every cursor rectangle: microseconds against the fraction of a millisecond
 * the PNG encode it avoids would cost.
 */
export function hashBytes(bytes) {
  let hash = 0x811c9dc5
  for (let i = 0; i < bytes.length; i++) {
    hash ^= bytes[i]
    // Multiply by the FNV prime, 16777619, without overflowing to a double.
    hash = Math.imul(hash, 0x01000193)
  }
  return (hash >>> 0).toString(36)
}
