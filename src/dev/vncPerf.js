// VNC benchmark. Dev builds only: termPerf.js runs it when started with
// `VITE_TERM_PERF=1 VITE_TERM_PERF_ONLY=vnc`, and Vite drops it from release
// builds.
//
// It opens a real VNC tab on a saved host and measures the client's side of
// every frame: size, which operations it carried, JPEG decode time, paint time
// and the gap since the previous frame. The remote's side -- how long the
// server took before the first byte of each update -- is the backend's
// `VNC frame timing` debug line; run with RUST_LOG=termdrop::vnc=debug and cut
// the log by the phase timestamps in the report.
//
// Input is pointer motion only, in a circle around the centre of the screen,
// with no button pressed: nothing is clicked, typed or dragged on the remote,
// and the path stays clear of hot corners, the menu bar and the Dock.

import { invoke } from '../utils/invoke.js'
import { OP } from '../utils/vncFrame.js'
import { TAB_KIND } from '../utils/tabKinds.js'

const sleep = ms => new Promise(r => setTimeout(r, ms))
const epoch = () => Date.now() / 1000
const HOST_NAME = import.meta.env.VITE_VNC_PERF_HOST || 'MacMini'
// The pixel mirror below runs on the main thread and skews timings, so it is
// opt-in: VITE_VNC_PERF_DIFF=1 to measure how much of each update changed.
const DIFF = Boolean(import.meta.env.VITE_VNC_PERF_DIFF)
const OP_NAMES = Object.fromEntries(Object.entries(OP).map(([k, v]) => [v, k]))

const percentile = (values, p) => {
  if (!values.length) return null
  const sorted = [...values].sort((a, b) => a - b)
  return sorted[Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length))]
}
const round = (n, digits = 1) => (n === null || n === undefined ? null : Number(n.toFixed(digits)))
const stats = values => ({
  median: round(percentile(values, 50), 2),
  p95: round(percentile(values, 95), 2),
  max: values.length ? round(Math.max(...values), 2) : null,
})

let frames = []
let attached = null

// A copy of the remote screen, kept only to answer one question: how much of
// what the server sends had actually changed? A pixel rectangle identical to
// what is already on screen is pure cost -- IPC, copy and paint for nothing.
let shadow = null
let shadowWidth = 0

function shadowApply(rects) {
  let sent = 0
  let changed = 0
  for (const r of rects) {
    if (r.op === OP.DESKTOP_SIZE || !shadow) {
      if (r.op === OP.DESKTOP_SIZE) {
        shadowWidth = r.width
        shadow = new Uint8ClampedArray(r.width * r.height * 4)
      }
      continue
    }
    if (r.op === OP.RAW_RGBA || r.op === OP.FILL) {
      const fill = r.op === OP.FILL ? r.payload : null
      for (let row = 0; row < r.height; row++) {
        const base = ((r.y + row) * shadowWidth + r.x) * 4
        for (let col = 0; col < r.width; col++) {
          const d = base + col * 4
          const s = (row * r.width + col) * 4
          const R = fill ? fill[0] : r.payload[s]
          const G = fill ? fill[1] : r.payload[s + 1]
          const B = fill ? fill[2] : r.payload[s + 2]
          sent++
          if (shadow[d] !== R || shadow[d + 1] !== G || shadow[d + 2] !== B) {
            changed++
            shadow[d] = R; shadow[d + 1] = G; shadow[d + 2] = B
          }
        }
      }
    } else if (r.op === OP.COPY_RECT) {
      const sx = r.payload[0] | (r.payload[1] << 8)
      const sy = r.payload[2] | (r.payload[3] << 8)
      const copy = new Uint8ClampedArray(r.width * r.height * 4)
      for (let row = 0; row < r.height; row++) {
        const from = ((sy + row) * shadowWidth + sx) * 4
        copy.set(shadow.subarray(from, from + r.width * 4), row * r.width * 4)
      }
      for (let row = 0; row < r.height; row++) {
        const to = ((r.y + row) * shadowWidth + r.x) * 4
        shadow.set(copy.subarray(row * r.width * 4, (row + 1) * r.width * 4), to)
      }
    }
  }
  return { sent, changed }
}

window.__vncPerf = {
  attach(panel) {
    attached = { ...panel, at: performance.now() }
    shadowWidth = panel.info.width
    shadow = new Uint8ClampedArray(panel.info.width * panel.info.height * 4)
  },
  frame(f) {
    const { sent, changed } = DIFF ? shadowApply(f.rects) : { sent: 0, changed: 0 }
    frames.push({
      bytes: f.bytes, decodeMs: f.decodeMs, applyMs: f.applyMs, gapMs: f.gapMs,
      ops: f.rects.map(r => r.op), pixelsSent: sent, pixelsChanged: changed,
      at: performance.now(),
    })
  },
}

/** Summarise the frames recorded while `fn` ran. */
async function measure(fn) {
  frames = []
  const start = performance.now()
  const extra = (await fn()) || {}
  const seconds = (performance.now() - start) / 1000
  const ops = {}
  for (const f of frames) for (const op of f.ops) ops[OP_NAMES[op] ?? op] = (ops[OP_NAMES[op] ?? op] || 0) + 1
  const bytes = frames.reduce((n, f) => n + f.bytes, 0)
  const pixelsSent = frames.reduce((n, f) => n + f.pixelsSent, 0)
  const pixelsChanged = frames.reduce((n, f) => n + f.pixelsChanged, 0)
  return {
    seconds: round(seconds, 2),
    frames: frames.length,
    fps: round(frames.length / seconds, 1),
    kb_per_frame: frames.length ? Math.round(bytes / frames.length / 1024) : 0,
    mb_total: round(bytes / 1e6, 2),
    ops,
    // Share of the pixels the server sent that differed from the screen.
    pixels_changed_pct: pixelsSent ? round((100 * pixelsChanged) / pixelsSent, 1) : null,
    mpixels_sent: round(pixelsSent / 1e6, 1),
    decode_ms: stats(frames.map(f => f.decodeMs)),
    paint_ms: stats(frames.map(f => f.applyMs)),
    gap_ms: stats(frames.slice(1).map(f => f.gapMs)),
    ...extra,
  }
}

/** Pointer motion at ~60 Hz along a circle around the centre. */
async function sweep(seconds) {
  const { width, height } = attached.info
  const cx = width / 2
  const cy = height / 2
  const r = Math.min(width, height) * 0.25
  const end = performance.now() + seconds * 1000
  let t = 0
  while (performance.now() < end) {
    const x = Math.round(cx + r * Math.cos(t))
    const y = Math.round(cy + r * Math.sin(t))
    invoke('vnc_pointer', { sessionId: attached.sessionId, x, y, buttonMask: 0 }).catch(() => {})
    t += 0.05
    await sleep(16)
  }
}

export async function runVncPerf(store, report, phase) {
  const host = store.hosts.find(h => h.name === HOST_NAME)
  if (!host) throw new Error(`no host named ${HOST_NAME}`)

  const opened = performance.now()
  frames = []
  attached = null
  store.openVncTab(host.id)
  const deadline = performance.now() + 30000
  while (!(attached && frames.length)) {
    if (performance.now() > deadline) throw new Error('no VNC frame within 30s')
    await sleep(20)
  }
  const first = frames[0]
  report.vnc = {
    host: HOST_NAME,
    width: attached.info.width,
    height: attached.info.height,
    security: attached.info.security,
    tunnelled: attached.info.tunnelled,
    connect_ms: round(attached.at - opened),
    first_frame_ms: round(first.at - opened),
    first_frame_kb: Math.round(first.bytes / 1024),
    first_frame_ops: [...new Set(first.ops.map(op => OP_NAMES[op]))],
  }

  await sleep(3000)
  await phase('vnc-idle', () => measure(() => sleep(5000)))
  await phase('vnc-pointer-sweep', () => measure(() => sweep(10)))
  await sleep(1000)
  await phase('vnc-full-refresh', () => measure(async () => {
    const times = []
    for (let i = 0; i < 5; i++) {
      const before = frames.length
      const t = performance.now()
      attached.refresh()
      while (frames.length === before && performance.now() - t < 5000) await sleep(5)
      // A full refresh can arrive as several updates; let it finish.
      await sleep(1500)
      const mine = frames.slice(before)
      times.push(mine.length ? mine[mine.length - 1].at - t : null)
    }
    return { refresh_to_last_frame_ms: times.map(v => round(v)) }
  }))

  // Correctness: what the canvas shows now against a full refresh, which the
  // backend sends whole. Pixels that differ are either the desktop genuinely
  // changing in between or a tile that was wrongly skipped.
  await phase('vnc-verify', async () => {
    const pixels = () => attached.canvas.getContext('2d').getImageData(0, 0, attached.info.width, attached.info.height).data
    await sleep(1500)
    // Snapshot right before the refresh and right after its first frame is
    // painted, so the desktop has as little time as possible to change in
    // between. A frame is painted before its record is pushed.
    const before = pixels()
    const count = frames.length
    attached.refresh()
    const t = performance.now()
    // An incremental update already in flight can arrive first; the refresh
    // is the frame that covers the whole screen.
    const full = attached.info.width * attached.info.height * 4 * 0.9
    const isRefresh = f => f.bytes >= full
    while (!frames.slice(count).some(isRefresh) && performance.now() - t < 10000) await sleep(1)
    const after = pixels()
    const refreshFrame = frames.slice(count).find(isRefresh)
    let differing = 0
    for (let i = 0; i < before.length; i += 4) {
      if (before[i] !== after[i] || before[i + 1] !== after[i + 1] || before[i + 2] !== after[i + 2]) differing++
    }
    return {
      differing_pixels: differing,
      differing_pct: round((100 * differing) / (before.length / 4), 3),
      refresh_frame_kb: refreshFrame ? Math.round(refreshFrame.bytes / 1024) : null,
      refresh_ms: refreshFrame ? round(refreshFrame.at - t) : null,
    }
  })

  report.vnc.finished_epoch = round(epoch(), 3)
  const tab = store.tabs.find(t => t.hostId === host.id && t.type === TAB_KIND.VNC)
  if (tab) store.closeServiceTab(tab.id)
}
