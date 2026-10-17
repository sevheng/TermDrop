import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { SearchAddon } from '@xterm/addon-search'
import { WebglAddon } from '@xterm/addon-webgl'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { Channel } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'
import { currentTerminalTheme } from './useTheme.js'
import { invoke } from '../utils/invoke.js'
import { toast } from '../utils/toast.js'

// Menlo and Monaco are macOS-only, so the old stack fell through to Courier
// New on Linux and Windows — the two platforms most of these sessions run on.
// JetBrains Mono ships with the app, so this resolves the same everywhere.
export const TERMINAL_FONT_FAMILY =
  '"JetBrains Mono", ui-monospace, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace'
const DEFAULT_FONT_SIZE = 14

// A terminal whose WebGL context is lost this many times inside the window
// stays on the DOM renderer for good, so a flaky driver cannot flicker.
const MAX_CONTEXT_LOSSES = 3
const CONTEXT_LOSS_WINDOW_MS = 60_000

// Logged once per app run, not once per tab.
let warnedNoWebgl = false

/**
 * Wait for the bundled terminal font before a terminal is created.
 *
 * xterm measures its cell size once, when it opens, and only re-measures on
 * the next resize. JetBrains Mono is not loaded until something asks for it,
 * so the first terminal measured the fallback font (15px rows instead of 17px
 * at 13px), and the first resize then computed its row count from the stale
 * size and drew past the bottom of its box -- over the system panel when that
 * was opened.
 */
export async function loadTerminalFont(fontSize) {
  if (typeof document === 'undefined' || !document.fonts) return
  await document.fonts.load(`${fontSize}px "JetBrains Mono"`).catch(() => {})
}

/** The saved terminal font size, or the default when unset. */
export async function loadTerminalFontSize() {
  const setting = await invoke('get_setting', { key: 'font_size' })
  return setting ? parseInt(setting) : DEFAULT_FONT_SIZE
}

/**
 * Create an xterm Terminal with the app's standard addons (fit, optional
 * search, clickable links), open it in `container`, and fit it once.
 *
 * Rendering: xterm 6 has only the DOM renderer built in and WebGL as an
 * addon (the Canvas addon TermDrop used to rely on is gone). WebGL used to
 * blank tabs because every hidden tab held its own context and WebKit drops
 * the oldest once too many are live. So a terminal holds a context **only
 * while visible**: the caller attaches the GPU renderer on activation and
 * detaches it on deactivation, and a hidden tab falls back to the DOM
 * renderer, which costs nothing under `display: none`. A lost context also
 * falls back to DOM until the next activation.
 *
 * Returns the terminal plus helpers for the two things every instance
 * needs: a binary data channel from the backend and a ResizeObserver that
 * refits the terminal. Input handling is left to the caller because the
 * shell and docker terminals batch keystrokes differently.
 */
export function createTerminalInstance(container, { fontSize, search = false }) {
  const term = new Terminal({
    cursorBlink: true,
    fontSize,
    fontFamily: TERMINAL_FONT_FAMILY,
    theme: currentTerminalTheme(),
  })

  const fitAddon = new FitAddon()
  const searchAddon = search ? new SearchAddon() : null
  const webLinksAddon = new WebLinksAddon((event, uri) => {
    event.preventDefault()
    openUrl(uri).catch((err) => {
      toast('Failed to open link: ' + err, 'error')
    })
  })
  term.loadAddon(fitAddon)
  if (searchAddon) term.loadAddon(searchAddon)
  term.loadAddon(webLinksAddon)

  term.open(container)
  fitAddon.fit()

  let resizeObserver = null
  let webgl = null
  let contextLosses = []
  let gpuDisabled = false

  /**
   * Render with WebGL. Safe to call repeatedly. Without WebGL (some VMs, WSL
   * without a GPU) the terminal simply stays on the DOM renderer.
   */
  function attachGpuRenderer() {
    if (webgl || gpuDisabled) return
    try {
      const addon = new WebglAddon()
      addon.onContextLoss(() => {
        const now = Date.now()
        contextLosses = contextLosses.filter(t => now - t < CONTEXT_LOSS_WINDOW_MS)
        contextLosses.push(now)
        if (contextLosses.length >= MAX_CONTEXT_LOSSES) {
          gpuDisabled = true
          console.warn('WebGL context lost repeatedly; this terminal stays on the DOM renderer')
        }
        detachGpuRenderer()
      })
      term.loadAddon(addon)
      webgl = addon
    } catch (err) {
      gpuDisabled = true
      if (!warnedNoWebgl) {
        warnedNoWebgl = true
        console.warn('WebGL unavailable; terminals use the DOM renderer:', err)
      }
    }
  }

  /** Release the WebGL context and fall back to the DOM renderer. */
  function detachGpuRenderer() {
    if (!webgl) return
    const addon = webgl
    webgl = null
    try {
      addon.dispose()
    } catch {
      // Already torn down with its context.
    }
    term.refresh(0, term.rows - 1)
  }

  /** Refit whenever `el` changes size. Safe to call repeatedly. */
  function observeResize(el) {
    if (!resizeObserver) {
      resizeObserver = new ResizeObserver(() => {
        fitAddon.fit()
      })
    }
    if (el) resizeObserver.observe(el)
  }

  /** Stop refitting until observeResize is called again. */
  function unobserveResize() {
    if (resizeObserver) {
      resizeObserver.disconnect()
      resizeObserver = null
    }
  }

  /**
   * Open a Tauri Channel that writes raw backend output into the terminal
   * and hand it to `command` along with `args`.
   */
  function openDataChannel(command, args) {
    const channel = new Channel()
    // Dev builds only: the terminal benchmark (src/dev/termPerf.js) watches
    // every message and when xterm has processed it. Vite strips this branch
    // from release builds.
    const perf = import.meta.env.DEV ? window.__termPerf?.attach(args.sessionId ?? args.ptySessionId, term) : null
    channel.onmessage = (message) => {
      // The backend sends raw bytes, which Tauri delivers as an ArrayBuffer
      // (inlined below 1 KB, fetched above it). The other shapes are what a
      // JSON-serialised channel produced, kept for the fallback event path.
      let data = null
      if (message instanceof ArrayBuffer) {
        data = new Uint8Array(message)
      } else if (message instanceof Uint8Array) {
        data = message
      } else if (Array.isArray(message)) {
        data = new Uint8Array(message)
      } else if (typeof message === 'string') {
        data = message
      }
      if (data === null) return
      if (perf) {
        perf.received()
        term.write(data, () => perf.processed(data))
      } else {
        term.write(data)
      }
    }
    invoke(command, { ...args, channel }).catch(() => {})
  }

  function dispose() {
    unobserveResize()
    detachGpuRenderer()
    term.dispose()
  }

  return {
    term,
    fitAddon,
    searchAddon,
    observeResize,
    unobserveResize,
    openDataChannel,
    attachGpuRenderer,
    detachGpuRenderer,
    dispose,
  }
}
