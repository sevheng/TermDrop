import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { SearchAddon } from '@xterm/addon-search'
import { CanvasAddon } from '@xterm/addon-canvas'
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

/** The saved terminal font size, or the default when unset. */
export async function loadTerminalFontSize() {
  const setting = await invoke('get_setting', { key: 'font_size' })
  return setting ? parseInt(setting) : DEFAULT_FONT_SIZE
}

/**
 * Create an xterm Terminal with the app's standard addons (fit, optional
 * search, canvas renderer, clickable links), open it in `container`, and
 * fit it once. The canvas renderer is deliberate: WebGL lost its context on
 * tab switches.
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
  const canvasAddon = new CanvasAddon()
  const webLinksAddon = new WebLinksAddon((event, uri) => {
    event.preventDefault()
    openUrl(uri).catch((err) => {
      toast('Failed to open link: ' + err, 'error')
    })
  })
  term.loadAddon(fitAddon)
  if (searchAddon) term.loadAddon(searchAddon)
  term.loadAddon(canvasAddon)
  term.loadAddon(webLinksAddon)

  term.open(container)
  fitAddon.fit()

  let resizeObserver = null

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
    channel.onmessage = (message) => {
      // Handle various possible data formats from Tauri Channel
      if (message instanceof Uint8Array) {
        term.write(message)
      } else if (Array.isArray(message)) {
        term.write(new Uint8Array(message))
      } else if (typeof message === 'string') {
        term.write(message)
      }
    }
    invoke(command, { ...args, channel }).catch(() => {})
  }

  function dispose() {
    unobserveResize()
    term.dispose()
  }

  return { term, fitAddon, searchAddon, observeResize, unobserveResize, openDataChannel, dispose }
}
