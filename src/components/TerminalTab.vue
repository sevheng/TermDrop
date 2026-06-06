<template>
  <div class="relative w-full h-full">
    <div ref="terminalContainer" class="w-full h-full" :class="terminalBgClass"></div>

    <!-- Search bar -->
    <div
      v-if="searchVisible"
      class="absolute top-2 right-2 bg-gray-800 border border-gray-600 rounded shadow-lg p-2 z-20 flex items-center gap-2"
    >
      <input
        ref="searchInput"
        v-model="searchQuery"
        type="text"
        placeholder="Find..."
        class="bg-gray-700 border border-gray-600 rounded px-2 py-1 text-sm text-white w-40 focus:outline-none focus:border-blue-500"
        @keydown.enter="findNext"
        @keydown.shift.enter="findPrevious"
        @keydown.esc="closeSearch"
      />
      <button
        @click="findPrevious"
        class="text-gray-300 hover:text-white px-1"
        title="Previous"
      >
        ↑
      </button>
      <button
        @click="findNext"
        class="text-gray-300 hover:text-white px-1"
        title="Next"
      >
        ↓
      </button>
      <label class="flex items-center gap-1 text-xs text-gray-300 cursor-pointer select-none">
        <input v-model="searchCaseSensitive" type="checkbox" class="accent-blue-500" />
        Aa
      </label>
      <button @click="closeSearch" class="text-gray-400 hover:text-white px-1">×</button>
    </div>

    <!-- Disconnect banner -->
    <div
      v-if="isDisconnected"
      class="absolute inset-0 bg-gray-900/90 flex flex-col items-center justify-center z-10"
    >
      <p class="text-red-400 text-lg font-semibold mb-4">Connection lost</p>
      <button
        @click="reconnect"
        :disabled="isReconnecting"
        class="px-6 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-gray-600 text-white rounded text-sm font-medium"
      >
        {{ isReconnecting ? 'Reconnecting...' : 'Reconnect' }}
      </button>
    </div>

    <!-- Context menu -->
    <div
      v-if="contextMenu.show"
      ref="contextMenuEl"
      class="fixed bg-gray-700 border border-gray-600 rounded shadow-lg py-1 z-50 min-w-[8rem]"
      :style="{ top: contextMenu.y + 'px', left: contextMenu.x + 'px' }"
    >
      <button @click="copySelection" class="block w-full text-left px-4 py-1.5 text-sm text-white hover:bg-gray-600">Copy</button>
      <button @click="pasteFromClipboard" class="block w-full text-left px-4 py-1.5 text-sm text-white hover:bg-gray-600">Paste</button>
      <button @click="selectAll" class="block w-full text-left px-4 py-1.5 text-sm text-white hover:bg-gray-600">Select All</button>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, onUnmounted, watch, nextTick } from 'vue'
import { Terminal } from 'xterm'
import { FitAddon } from 'xterm-addon-fit'
import { SearchAddon } from 'xterm-addon-search'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import 'xterm/css/xterm.css'

const props = defineProps({
  sessionId: {
    type: String,
    required: true,
  },
})

const terminalContainer = ref(null)
const contextMenuEl = ref(null)
const searchInput = ref(null)
let term = null
let fitAddon = null
let searchAddon = null
let unlistenData = null
let unlistenError = null
let unlistenConnected = null
let unlistenDisconnected = null
let unlistenReconnected = null
let resizeObserver = null

const isDisconnected = ref(false)
const isReconnecting = ref(false)
const contextMenu = ref({ show: false, x: 0, y: 0 })
const terminalBgClass = ref('bg-gray-900')

const searchVisible = ref(false)
const searchQuery = ref('')
const searchCaseSensitive = ref(false)

const darkTheme = {
  background: '#111827',
  foreground: '#e5e7eb',
  cursor: '#e5e7eb',
  selectionBackground: '#374151',
  black: '#1f2937',
  red: '#ef4444',
  green: '#22c55e',
  yellow: '#eab308',
  blue: '#3b82f6',
  magenta: '#a855f7',
  cyan: '#06b6d4',
  white: '#f3f4f6',
}

const lightTheme = {
  background: '#ffffff',
  foreground: '#111827',
  cursor: '#111827',
  selectionBackground: '#bfdbfe',
  black: '#1f2937',
  red: '#dc2626',
  green: '#16a34a',
  yellow: '#ca8a04',
  blue: '#2563eb',
  magenta: '#9333ea',
  cyan: '#0891b2',
  white: '#f3f4f6',
}

function getThemeColors(themeName) {
  return themeName === 'light' ? lightTheme : darkTheme
}

function applySettings(settings) {
  if (!term) return
  if (settings.fontSize !== undefined) {
    term.options.fontSize = settings.fontSize
  }
  if (settings.theme !== undefined) {
    const colors = getThemeColors(settings.theme)
    term.options.theme = colors
    terminalBgClass.value = settings.theme === 'light' ? 'bg-white' : 'bg-gray-900'
  }
  if (fitAddon) {
    setTimeout(() => fitAddon.fit(), 50)
  }
}

function openSearch() {
  searchVisible.value = true
  nextTick(() => searchInput.value?.focus())
}

function closeSearch() {
  searchVisible.value = false
  searchQuery.value = ''
  if (searchAddon) {
    searchAddon.clearDecorations()
  }
  term?.focus()
}

function findNext() {
  if (!searchAddon || !searchQuery.value) return
  searchAddon.findNext(searchQuery.value, { caseSensitive: searchCaseSensitive.value })
}

function findPrevious() {
  if (!searchAddon || !searchQuery.value) return
  searchAddon.findPrevious(searchQuery.value, { caseSensitive: searchCaseSensitive.value })
}

async function copySelection() {
  contextMenu.value.show = false
  const selection = term.getSelection()
  if (selection) {
    try {
      await navigator.clipboard.writeText(selection)
    } catch (e) {
      console.warn('Copy failed:', e)
    }
  }
}

async function pasteFromClipboard() {
  contextMenu.value.show = false
  try {
    const text = await navigator.clipboard.readText()
    if (text) {
      term.paste(text)
    }
  } catch (e) {
    console.warn('Paste failed:', e)
  }
}

function selectAll() {
  contextMenu.value.show = false
  term.selectAll()
}

async function showContextMenu(event) {
  event.preventDefault()
  contextMenu.value = {
    show: true,
    x: event.clientX,
    y: event.clientY,
  }
  await nextTick()
  const el = contextMenuEl.value
  if (el) {
    const rect = el.getBoundingClientRect()
    const vw = window.innerWidth
    const vh = window.innerHeight
    let x = contextMenu.value.x
    let y = contextMenu.value.y
    if (x + rect.width > vw) x = vw - rect.width - 8
    if (y + rect.height > vh) y = vh - rect.height - 8
    if (x < 8) x = 8
    if (y < 8) y = 8
    contextMenu.value.x = x
    contextMenu.value.y = y
  }
}

function onWindowClick() {
  contextMenu.value.show = false
}

function onWindowContextMenu() {
  contextMenu.value.show = false
}

function onSettingsChanged(event) {
  applySettings(event.detail)
}

onMounted(async () => {
  const fontSizeSetting = await invoke('get_setting', { key: 'font_size' })
  const fontSize = fontSizeSetting ? parseInt(fontSizeSetting) : 14
  const themeSetting = await invoke('get_setting', { key: 'theme' })
  const themeName = themeSetting || 'dark'
  terminalBgClass.value = themeName === 'light' ? 'bg-white' : 'bg-gray-900'

  term = new Terminal({
    cursorBlink: true,
    fontSize,
    fontFamily: 'Menlo, Monaco, "Courier New", monospace',
    theme: getThemeColors(themeName),
  })

  fitAddon = new FitAddon()
  searchAddon = new SearchAddon()
  term.loadAddon(fitAddon)
  term.loadAddon(searchAddon)
  term.open(terminalContainer.value)
  fitAddon.fit()

  // Delayed fit to handle flex layout settling
  setTimeout(() => {
    if (fitAddon) fitAddon.fit()
  }, 150)

  // Custom keyboard shortcuts
  term.attachCustomKeyEventHandler((e) => {
    if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === 'c') {
      copySelection()
      return false
    }
    if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === 'v') {
      navigator.clipboard.readText().then(text => {
        if (text) term.paste(text)
      }).catch(err => console.warn('Paste failed:', err))
      return false
    }
    if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === 'a') {
      selectAll()
      return false
    }
    if (e.ctrlKey && e.key.toLowerCase() === 'f') {
      openSearch()
      return false
    }
    if (e.key === 'Escape' && searchVisible.value) {
      closeSearch()
      return false
    }
    return true
  })

  // Right-click context menu
  terminalContainer.value.addEventListener('contextmenu', showContextMenu)

  // Listen for SSH data
  unlistenData = await listen('ssh-data', (event) => {
    const payload = event.payload
    if (typeof payload === 'object' && payload.session_id === props.sessionId) {
      term.write(payload.data)
    } else if (typeof payload === 'string') {
      term.write(payload)
    }
  })

  // Listen for SSH errors
  unlistenError = await listen('ssh-error', (event) => {
    const payload = event.payload
    if (typeof payload === 'object' && payload.session_id === props.sessionId) {
      term.writeln(`\r\n\x1b[31mError: ${payload.error}\x1b[0m`)
    }
  })

  // Listen for SSH connection established
  unlistenConnected = await listen('ssh-connected', (event) => {
    if (event.payload === props.sessionId) {
      setTimeout(() => {
        if (fitAddon) fitAddon.fit()
      }, 100)
    }
  })

  // Listen for SSH disconnections
  unlistenDisconnected = await listen('ssh-disconnected', (event) => {
    if (event.payload === props.sessionId) {
      isDisconnected.value = true
    }
  })

  // Listen for SSH reconnections
  unlistenReconnected = await listen('ssh-reconnected', (event) => {
    if (event.payload === props.sessionId) {
      isDisconnected.value = false
      isReconnecting.value = false
      term.clear()
      setTimeout(() => {
        if (fitAddon) fitAddon.fit()
      }, 100)
    }
  })

  // Send keystrokes to SSH
  term.onData((data) => {
    invoke('ssh_write', { sessionId: props.sessionId, data }).catch((err) => {
      console.error('ssh_write failed:', err)
    })
  })

  // Handle resize
  resizeObserver = new ResizeObserver(() => {
    if (fitAddon) {
      fitAddon.fit()
    }
  })
  resizeObserver.observe(terminalContainer.value)

  // Listen for settings changes
  window.addEventListener('terminal-settings-changed', onSettingsChanged)
  window.addEventListener('click', onWindowClick)
  window.addEventListener('contextmenu', onWindowContextMenu, true)
})

async function reconnect() {
  isReconnecting.value = true
  try {
    await invoke('ssh_reconnect', { sessionId: props.sessionId })
  } catch (err) {
    console.error('Reconnect failed:', err)
    isReconnecting.value = false
  }
}

onUnmounted(() => {
  if (unlistenData) unlistenData()
  if (unlistenError) unlistenError()
  if (unlistenConnected) unlistenConnected()
  if (unlistenDisconnected) unlistenDisconnected()
  if (unlistenReconnected) unlistenReconnected()
  if (resizeObserver) resizeObserver.disconnect()
  if (term) term.dispose()
  window.removeEventListener('terminal-settings-changed', onSettingsChanged)
  window.removeEventListener('click', onWindowClick)
  window.removeEventListener('contextmenu', onWindowContextMenu, true)
  if (terminalContainer.value) {
    terminalContainer.value.removeEventListener('contextmenu', showContextMenu)
  }
})

// Handle sessionId changes
watch(() => props.sessionId, (newId, oldId) => {
  if (term && newId !== oldId) {
    term.clear()
  }
})
</script>
