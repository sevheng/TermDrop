<template>
  <div class="flex flex-col h-full bg-canvas">
    <PanelHeader
      :title="host?.name || 'Remote desktop'"
      :icon="Monitor"
      icon-class="text-vnc"
      :subtitle="displayTarget"
      subtitle-mono
      :meta="size.width ? `${size.width}×${size.height}` : ''"
    >
      <template #badges>
        <span
          v-if="info?.tunnelled"
          class="px-1.5 py-0.5 rounded text-2xs bg-active text-syn-cyan"
        >
          tunnelled
        </span>
        <span
          v-if="viewOnly"
          class="px-1.5 py-0.5 rounded text-2xs bg-warn-bg text-warn"
        >
          view only
        </span>
        <span
          v-if="showStats"
          class="px-1.5 py-0.5 rounded text-2xs bg-raised text-ink-3 tabular-nums"
          :title="statsHint"
        >
          {{ stats.fps }}fps · {{ stats.gap }}ms gap · {{ stats.paint }}ms paint ·
          {{ stats.perFrame }}
        </span>
      </template>
      <template #actions>
        <IconButton :icon="RefreshCw" label="Refresh screen" :disabled="!connected" @click="refresh" />
        <IconButton
          :icon="viewOnly ? EyeOff : Eye"
          :label="viewOnly ? 'Allow input' : 'View only'"
          :disabled="!connected"
          @click="toggleViewOnly"
        />
        <IconButton
          :icon="Maximize2"
          :label="fitMode === FIT ? 'Actual size' : 'Fit to window'"
          @click="toggleFit"
        />
        <IconButton
          :icon="Command"
          label="Send Ctrl+Alt+Delete"
          :disabled="!connected || viewOnly"
          @click="sendCtrlAltDel"
        />
        <IconButton
          :icon="ClipboardCopy"
          label="Send clipboard to remote"
          :disabled="!connected || viewOnly"
          @click="sendClipboard"
        />
      </template>
    </PanelHeader>

    <div
      v-if="error"
      class="m-3 p-3 rounded border border-bad-line bg-bad-bg text-bad text-xs flex items-start gap-3"
    >
      <span class="flex-1">{{ error }}</span>
      <button class="px-2 py-1 rounded bg-raised text-ink hover:bg-input" @click="connect">
        Reconnect
      </button>
    </div>

    <div
      ref="viewport"
      class="flex-1 relative flex items-center justify-center bg-canvas"
      :class="fitMode === FIT ? 'overflow-hidden' : 'overflow-auto'"
    >
      <div v-if="connecting" class="flex items-center gap-2 text-ink-2 text-xs">
        <Loader2 :size="14" class="animate-spin" />
        Connecting…
      </div>

      <canvas
        v-show="connected && !connecting"
        ref="canvasEl"
        tabindex="0"
        class="block outline-none"
        :class="[scale >= 1 ? 'pixelated' : '', hasFocus ? 'ring-1 ring-accent' : '']"
        :style="{
          width: cssW + 'px',
          height: cssH + 'px',
          cursor: viewOnly ? 'default' : cursorStyle,
        }"
        @pointermove="handlers.onPointerMove"
        @pointerdown="handlers.onPointerDown"
        @pointerup="handlers.onPointerUp"
        @pointercancel="handlers.onPointerCancel"
        @wheel="handlers.onWheel"
        @keydown="handlers.onKeyDown"
        @keyup="handlers.onKeyUp"
        @focus="handlers.onFocus"
        @blur="handlers.onBlur"
        @contextmenu="handlers.onContextMenu"
      ></canvas>
    </div>
  </div>
</template>

<script setup>
/**
 * A remote desktop on a canvas.
 *
 * Two rules govern the rendering, and both are easy to get subtly wrong:
 *
 * **The backing store is never scaled.** `putImageData` ignores the canvas
 * transform, so a scaled backing store would silently misplace every raw
 * rectangle while leaving JPEG and CopyRect ones correct — which looks like a
 * decoder bug and is not one. Scaling is CSS only.
 *
 * **A frame is applied whole, in wire order.** JPEG rectangles decode
 * asynchronously, so every bitmap for a frame is awaited before *any* of the
 * frame is drawn. Applying them as they resolve would let a later CopyRect
 * overtake the JPEG it copies from.
 */
import { ref, computed, watch, onMounted, onBeforeUnmount, nextTick } from 'vue'
import { Channel } from '@tauri-apps/api/core'
import {
  Monitor,
  RefreshCw,
  Eye,
  EyeOff,
  Maximize2,
  Command,
  ClipboardCopy,
  Loader2,
} from 'lucide-vue-next'
import { readText } from '@tauri-apps/plugin-clipboard-manager'
import { invoke } from '../utils/invoke.js'
import { toast } from '../utils/toast.js'
import { useConnectionStore } from '../stores/connection.js'
import { useListenerGroup } from '../composables/useListenerGroup.js'
import { useVncInput } from '../composables/useVncInput.js'
import { parseFrame, fillColour, hashBytes, OP } from '../utils/vncFrame.js'
import { vncLayout, FIT, ACTUAL } from '../utils/vncLayout.js'
import { vncDisplayTarget } from '../utils/vncTarget.js'
import { CTRL_ALT_DELETE } from '../utils/keysym.js'
import PanelHeader from './PanelHeader.vue'
import IconButton from './IconButton.vue'

const props = defineProps({
  hostId: { type: Number, required: true },
  isActive: { type: Boolean, default: true },
  /** The store's tab, whose `connected` flag this panel keeps true to the session. */
  tabId: { type: String, default: null },
})

const store = useConnectionStore()
const listeners = useListenerGroup()

const host = computed(() => store.hosts.find(h => h.id === props.hostId))
const displayTarget = computed(() => vncDisplayTarget(host.value))

const sessionId = ref(null)
const info = ref(null)
const connecting = ref(false)
const connected = ref(false)
const error = ref('')
const viewOnly = ref(false)
const fitMode = ref(FIT)
const size = ref({ width: 0, height: 0 })
/**
 * Frame statistics, off by default.
 *
 * "It feels slow" is how this feature shipped with a 250ms stall in it. These
 * are the numbers that answer it instead. Updated once a second rather than
 * per frame, so the readout's own reactivity never lands on the frame path.
 */
const showStats = ref(false)
const stats = ref({ fps: 0, gap: 0, paint: 0, perFrame: '—' })
const statsHint =
  'Frames per second, the gap between one frame arriving and the next, the ' +
  'time spent painting it, and its size. The gap minus the paint is time ' +
  'spent waiting on the remote. Run with RUST_LOG=termdrop::vnc=debug to see ' +
  'that split exactly.'

/**
 * When the previous frame arrived.
 *
 * Measures the gap between frames rather than a round trip: requests are
 * pipelined, so the next one is already on the wire before this frame is
 * acknowledged and there is no round trip left to measure from here.
 */
let lastFrameAt = 0
/** Serializes frame application; see `attachChannel`. */
let frameChain = Promise.resolve()
let window1s = { frames: 0, bytes: 0, paint: 0, gap: 0, since: 0 }
let statsTimer = null

/** Fold one frame into the current second's totals. */
function recordFrame(bytes, paintMs, gapMs) {
  window1s.frames += 1
  window1s.bytes += bytes
  window1s.paint += paintMs
  window1s.gap += gapMs
}

function publishStats() {
  const { frames, bytes, paint, gap } = window1s
  window1s = { frames: 0, bytes: 0, paint: 0, gap: 0, since: 0 }
  if (!frames) {
    lastFrameAt = 0
    stats.value = { fps: 0, gap: 0, paint: 0, perFrame: 'idle' }
    return
  }
  const perFrame = bytes / frames
  stats.value = {
    fps: frames,
    gap: Math.round(gap / frames),
    paint: Math.round((paint / frames) * 10) / 10,
    perFrame: perFrame > 512 * 1024
      ? `${(perFrame / (1024 * 1024)).toFixed(1)}MB/frame`
      : `${Math.round(perFrame / 1024)}KB/frame`,
  }
}

/** The remote pointer, drawn by the compositor rather than into the canvas. */
const cursorStyle = ref('default')
/**
 * The sequence number of a frame painted while paused.
 *
 * Held so the stream can resume on the next frame rather than waiting out the
 * backend's acknowledgement timeout.
 */
let withheldAck = null

const canvasEl = ref(null)
const viewport = ref(null)
const viewportSize = ref({ width: 0, height: 0 })
let ctx = null
let channel = null
let resizeObserver = null

const { hasFocus, releaseAllKeys, handlers } = useVncInput({
  sessionId,
  canvasRef: canvasEl,
  size,
  viewOnly,
  onReleaseKeyboard: () => canvasEl.value?.blur(),
})

// -- layout -----------------------------------------------------------------

const layout = computed(() =>
  vncLayout({
    fbW: size.value.width,
    fbH: size.value.height,
    viewportW: viewportSize.value.width,
    viewportH: viewportSize.value.height,
    mode: fitMode.value,
    dpr: window.devicePixelRatio || 1,
  }),
)
const cssW = computed(() => layout.value.cssW)
const cssH = computed(() => layout.value.cssH)
const scale = computed(() => layout.value.scale)

function toggleFit() {
  fitMode.value = fitMode.value === FIT ? ACTUAL : FIT
  invoke('set_setting', { key: 'vnc.fitMode', value: fitMode.value }).catch(() => {})
}

// -- connection -------------------------------------------------------------

async function connect() {
  if (!host.value?.vnc_host || connecting.value) return
  error.value = ''
  connecting.value = true
  try {
    const result = await invoke('vnc_connect', { hostId: props.hostId })
    info.value = result
    sessionId.value = result.session_id
    size.value = { width: result.width, height: result.height }
    viewOnly.value = result.view_only
    connected.value = true
    await nextTick()
    prepareCanvas()
    await attachChannel()
  } catch (e) {
    error.value = String(e)
    connected.value = false
  } finally {
    connecting.value = false
  }
}

async function disconnect() {
  releaseAllKeys()
  const id = sessionId.value
  sessionId.value = null
  connected.value = false
  channel = null
  if (id) await invoke('vnc_disconnect', { sessionId: id }).catch(() => {})
}

/**
 * Register the frame channel.
 *
 * The backend requests no frames until this exists, which removes the
 * dropped-first-frame race rather than papering over it with a retry.
 */
async function attachChannel() {
  channel = new Channel()
  // Strictly serialized, and not optional: `onFrame` awaits `createImageBitmap`
  // for JPEG rectangles, so with more than one frame in flight a later frame
  // could finish its decode first and paint over an earlier one. Chaining makes
  // the order the wire order regardless of how many are outstanding.
  frameChain = Promise.resolve()
  channel.onmessage = message => {
    frameChain = frameChain.then(() => onFrame(message)).catch(() => {})
  }
  await invoke('vnc_open_frame_channel', { sessionId: sessionId.value, channel })
}

function prepareCanvas() {
  const canvas = canvasEl.value
  if (!canvas) return
  // Exactly the framebuffer's size, always. See the note at the top.
  canvas.width = size.value.width
  canvas.height = size.value.height
  ctx = canvas.getContext('2d', { alpha: false, desynchronized: true })
  if (ctx) {
    ctx.fillStyle = '#000'
    ctx.fillRect(0, 0, canvas.width, canvas.height)
  }
}

// -- rendering --------------------------------------------------------------

async function onFrame(message) {
  const buffer = toArrayBuffer(message)
  if (!buffer || !ctx) return

  const startedAt = performance.now()
  const gap = lastFrameAt ? startedAt - lastFrameAt : 0
  lastFrameAt = startedAt

  let parsed
  try {
    parsed = parseFrame(buffer)
  } catch (e) {
    error.value = `Corrupt frame from the server: ${e.message}`
    await disconnect()
    return
  }

  // Start every JPEG decoding at once, then wait for all of them, so the
  // whole frame can be applied in one synchronous pass in wire order.
  const bitmaps = new Map()
  const decoding = []
  for (const [index, rect] of parsed.rects.entries()) {
    if (rect.op !== OP.JPEG) continue
    const blob = new Blob([rect.payload], { type: 'image/jpeg' })
    decoding.push(
      createImageBitmap(blob)
        .then(bitmap => bitmaps.set(index, bitmap))
        .catch(() => {}),
    )
  }
  if (decoding.length) await Promise.all(decoding)

  applyRects(parsed.rects, bitmaps)

  if (showStats.value) {
    recordFrame(buffer.byteLength, performance.now() - startedAt, gap)
  }

  ackFrame(parsed.seq)
}

/**
 * Tell the backend it may request the next frame.
 *
 * This is the whole flow-control mechanism: no further update is requested
 * until it arrives, so the server can never outrun the canvas.
 *
 * It used to be wrapped in a `requestAnimationFrame` on the theory that this
 * waited for the compositor. It did not — rAF callbacks run *before* paint,
 * and the context is `desynchronized` — so all it added was 0-16.7ms to every
 * frame's round trip. What it did provide, by accident, was the pause when the
 * window is hidden; that is now a stated condition instead.
 */
function ackFrame(seq) {
  if (!sessionId.value) return
  // `isActive` matters as much as visibility: the panel is toggled with
  // `v-show`, so a VNC tab sitting behind a terminal stays mounted and would
  // otherwise keep pulling and decoding frames nobody can see.
  if (!props.isActive || document.visibilityState === 'hidden') {
    withheldAck = seq
    return
  }
  withheldAck = null
  invoke('vnc_frame_ack', { sessionId: sessionId.value, seq }).catch(() => {})
}

/**
 * Tell the backend to stop or resume requesting frames.
 *
 * Withholding acknowledgements is not enough on its own: the backend expires
 * an unanswered frame after its timeout and asks for the next one, so a hidden
 * tab on a busy desktop kept streaming at about four frames a second.
 */
function setPaused(paused) {
  if (!sessionId.value) return
  invoke('vnc_set_paused', { sessionId: sessionId.value, paused }).catch(() => {})
}

/** Resume a stream that was paused mid-frame, without waiting for the timeout. */
function resumeStream() {
  setPaused(false)
  if (withheldAck !== null) ackFrame(withheldAck)
  refresh()
}

function applyRects(rects, bitmaps) {
  const canvas = canvasEl.value
  for (const [index, rect] of rects.entries()) {
    switch (rect.op) {
      case OP.RAW_RGBA: {
        // A view onto the received buffer, so this is one copy into the
        // backing store with no decode and no intermediate allocation.
        const pixels = new Uint8ClampedArray(
          rect.payload.buffer,
          rect.payload.byteOffset,
          rect.payload.byteLength,
        )
        ctx.putImageData(new ImageData(pixels, rect.width, rect.height), rect.x, rect.y)
        break
      }
      case OP.COPY_RECT: {
        const sx = rect.payload[0] | (rect.payload[1] << 8)
        const sy = rect.payload[2] | (rect.payload[3] << 8)
        // A canvas self-blit is well defined, and is why dragging a window is
        // nearly free.
        ctx.drawImage(canvas, sx, sy, rect.width, rect.height, rect.x, rect.y, rect.width, rect.height)
        break
      }
      case OP.FILL:
        ctx.fillStyle = fillColour(rect.payload)
        ctx.fillRect(rect.x, rect.y, rect.width, rect.height)
        break
      case OP.JPEG: {
        const bitmap = bitmaps.get(index)
        if (bitmap) {
          ctx.drawImage(bitmap, rect.x, rect.y)
          bitmap.close()
        }
        break
      }
      case OP.CURSOR: {
        // A CSS cursor, so the pointer is drawn by the compositor and tracks
        // the mouse with no round trip. Drawing it into the framebuffer
        // instead would make it lag by a whole frame.
        applyCursor(rect)
        break
      }
      case OP.DESKTOP_SIZE:
        // Always the frame's last rectangle: resizing clears the canvas, so
        // anything after it would be painted and thrown away.
        size.value = { width: rect.width, height: rect.height }
        prepareCanvas()
        break
      default:
        break
    }
  }
}

/**
 * Turn a cursor rectangle into a CSS cursor.
 *
 * A zero-sized one means the remote is hiding the pointer entirely, which is
 * what a full-screen video player or a kiosk does.
 */
/**
 * Cursor images we have already encoded, keyed by a hash of their pixels.
 *
 * `toDataURL` is a synchronous PNG encode plus base64, and it sits on the
 * serialized frame path — the acknowledgement, and so the next request, waits
 * for it. macOS cycles through the same dozen shapes endlessly (arrow, beam,
 * resize handles, link pointer), so caching removes essentially all of it.
 */
const cursorCache = new Map()
const CURSOR_CACHE_MAX = 32

function applyCursor(rect) {
  if (!rect.width || !rect.height) {
    setCursor('none')
    return
  }

  const key = `${rect.width}x${rect.height}:${hashBytes(rect.payload)}`
  let url = cursorCache.get(key)
  if (!url) {
    const scratch = document.createElement('canvas')
    scratch.width = rect.width
    scratch.height = rect.height
    const scratchCtx = scratch.getContext('2d')
    const pixels = new Uint8ClampedArray(
      rect.payload.buffer,
      rect.payload.byteOffset,
      rect.payload.byteLength,
    )
    scratchCtx.putImageData(new ImageData(pixels, rect.width, rect.height), 0, 0)
    url = scratch.toDataURL()

    // Plain FIFO eviction; a Map keeps insertion order.
    if (cursorCache.size >= CURSOR_CACHE_MAX) {
      cursorCache.delete(cursorCache.keys().next().value)
    }
    cursorCache.set(key, url)
  }

  // The hotspot is not part of the key: the same bitmap can arrive with a
  // different one.
  setCursor(`url(${url}) ${rect.x} ${rect.y}, default`)
}

/**
 * Write the cursor only when it actually changed.
 *
 * Assigning an identical string still triggers Vue's effect and re-sets the
 * style attribute, which dirties layout — and a dirty layout is what makes
 * every subsequent `getBoundingClientRect` expensive.
 */
function setCursor(next) {
  if (next !== cursorStyle.value) cursorStyle.value = next
}

/** Tauri hands a Raw channel payload over as an ArrayBuffer. */
function toArrayBuffer(message) {
  if (message instanceof ArrayBuffer) return message
  if (ArrayBuffer.isView(message)) return message.buffer
  // A small Raw payload arrives already wrapped; anything else is not ours.
  return null
}

// -- actions ----------------------------------------------------------------

function refresh() {
  if (sessionId.value) invoke('vnc_refresh', { sessionId: sessionId.value }).catch(() => {})
}

function toggleViewOnly() {
  viewOnly.value = !viewOnly.value
  if (viewOnly.value) releaseAllKeys()
  if (sessionId.value) {
    invoke('vnc_set_view_only', {
      sessionId: sessionId.value,
      viewOnly: viewOnly.value,
    }).catch(() => {})
  }
}

function sendCtrlAltDel() {
  if (!sessionId.value) return
  invoke('vnc_key_sequence', { sessionId: sessionId.value, keysyms: CTRL_ALT_DELETE }).catch(() => {})
}

async function sendClipboard() {
  if (!sessionId.value) return
  try {
    const text = await readText()
    if (!text) return
    const lossy = await invoke('vnc_send_clipboard', { sessionId: sessionId.value, text })
    if (lossy) {
      toast('Some characters could not be sent: VNC clipboard is Latin-1 only', 'warning')
    }
  } catch (e) {
    toast(`Could not send the clipboard: ${e}`, 'error')
  }
}

// -- lifecycle --------------------------------------------------------------

function measure() {
  const el = viewport.value
  if (el) viewportSize.value = { width: el.clientWidth, height: el.clientHeight }
}

onMounted(async () => {
  measure()
  resizeObserver = new ResizeObserver(measure)
  if (viewport.value) resizeObserver.observe(viewport.value)

  const stored = await invoke('get_setting', { key: 'vnc.fitMode' }).catch(() => null)
  if (stored === ACTUAL || stored === FIT) fitMode.value = stored

  // No toggle in the UI yet: set it by hand with the setting when diagnosing.
  showStats.value =
    (await invoke('get_setting', { key: 'vnc.showStats' }).catch(() => null)) === 'true'
  if (showStats.value) statsTimer = setInterval(publishStats, 1000)

  await listeners.listen('vnc-resize', e => {
    if (e.payload?.sessionId !== sessionId.value) return
    size.value = { width: e.payload.width, height: e.payload.height }
  })
  await listeners.listen('vnc-error', e => {
    if (e.payload?.sessionId !== sessionId.value) return
    error.value = e.payload.error
    connected.value = false
  })
  await listeners.listen('vnc-disconnected', e => {
    if (e.payload !== sessionId.value) return
    connected.value = false
  })
  await listeners.listen('vnc-bell', sessionIdOf => {
    // The remote asked for attention. A toast rather than a sound: an app that
    // beeps unprompted from a background tab is not a welcome one.
    if (sessionIdOf.payload !== sessionId.value || !props.isActive) return
    toast(`${host.value?.name || 'Remote desktop'} is asking for attention`, 'info')
  })
  await listeners.listen('vnc-clipboard', e => {
    // Only while the tab is in front: a background tab quietly replacing the
    // system clipboard is genuinely nasty behaviour.
    if (e.payload?.sessionId !== sessionId.value || !props.isActive) return
    import('@tauri-apps/plugin-clipboard-manager')
      .then(m => m.writeText(e.payload.text))
      .catch(() => {})
  })

  document.addEventListener('visibilitychange', onVisibility)
  await connect()
})

function onVisibility() {
  if (document.visibilityState === 'hidden') setPaused(true)
  else if (props.isActive && connected.value) resumeStream()
}

onBeforeUnmount(() => {
  resizeObserver?.disconnect()
  document.removeEventListener('visibilitychange', onVisibility)
  clearInterval(statsTimer)
  disconnect()
})

// A backgrounded tab pauses the request pump, so the server stops sending.
// Coming back needs a full refresh, since the incremental updates missed while
// away were never sent.
watch(
  () => props.isActive,
  active => {
    if (active && connected.value) resumeStream()
    if (!active) {
      setPaused(true)
      releaseAllKeys()
    }
  },
)

// The host row reads the store, not this panel. Immediate, so the row is not
// green while the first connection is still being attempted.
watch(
  connected,
  value => {
    if (props.tabId) store.setTabConnected(props.tabId, value)
  },
  { immediate: true },
)

// The keyed v-for can reuse this instance for a different host.
watch(
  () => props.hostId,
  async () => {
    await disconnect()
    info.value = null
    error.value = ''
    size.value = { width: 0, height: 0 }
    await connect()
  },
)
</script>

<style scoped>
.pixelated {
  image-rendering: pixelated;
}
</style>
