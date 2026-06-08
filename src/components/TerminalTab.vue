<template>
  <div class="relative w-full h-full flex flex-col">
    <div ref="terminalContainer" class="flex-1 min-h-0" :class="terminalBgClass"></div>

    <!-- System Status Bar -->
    <div
      v-if="props.hostId"
      class="shrink-0 border-t border-gray-200 bg-gray-50 dark:bg-gray-800 dark:border-gray-700 transition-all"
      :class="statusExpanded ? '' : ''"
    >
      <div class="flex items-center justify-between px-2 py-0.5">
        <div class="flex items-center gap-3 overflow-x-auto">
          <div
            v-if="status.os"
            class="flex items-center gap-1 text-[10px] text-gray-600 dark:text-gray-400 max-w-[140px] cursor-default"
            @mouseenter="showTooltip($event, status.os)"
            @mouseleave="hideTooltip"
          >
            <Monitor :size="10" class="text-cyan-500 shrink-0" />
            <span class="truncate">{{ status.os }}</span>
          </div>
          <div v-if="status.load" class="flex items-center gap-1 text-[10px] text-gray-600 dark:text-gray-400 whitespace-nowrap">
            <Cpu :size="10" class="text-blue-500" />
            <span class="font-medium">CPU:</span>
            <span>{{ status.load }}<span v-if="status.cores"> / {{ status.cores }} cores</span></span>
          </div>
          <div v-if="status.ram" class="flex items-center gap-1 text-[10px] text-gray-600 dark:text-gray-400 whitespace-nowrap">
            <MemoryStick :size="10" class="text-green-500" />
            <span class="font-medium">RAM:</span>
            <span>{{ status.ram }}</span>
          </div>
          <div v-if="status.disk" class="flex items-center gap-1 text-[10px] text-gray-600 dark:text-gray-400 whitespace-nowrap">
            <HardDrive :size="10" class="text-orange-500" />
            <span class="font-medium">Disk:</span>
            <span>{{ status.disk }}</span>
          </div>
          <div v-if="status.uptime" class="flex items-center gap-1 text-[10px] text-gray-600 dark:text-gray-400 whitespace-nowrap">
            <Clock :size="10" class="text-purple-500" />
            <span class="font-medium">Up:</span>
            <span>{{ status.uptime }}</span>
          </div>
          <span v-if="statusError" class="text-[10px] text-red-500">{{ statusError }}</span>
        </div>
        <button
          @click="statusExpanded = !statusExpanded"
          class="text-gray-400 hover:text-gray-600 dark:hover:text-gray-300 shrink-0 ml-2"
          :title="statusExpanded ? 'Hide status' : 'Show status'"
        >
          <ChevronUp v-if="statusExpanded" :size="12" />
          <ChevronDown v-else :size="12" />
        </button>
      </div>
    </div>

    <!-- Custom tooltip -->
    <div
      v-if="tooltip.show"
      class="fixed z-50 px-2 py-1 bg-gray-800 text-white text-xs rounded shadow-lg pointer-events-none whitespace-nowrap dark:bg-gray-700"
      :style="{ left: tooltip.x + 'px', top: tooltip.y + 'px' }"
    >
      {{ tooltip.text }}
    </div>

    <!-- Search bar -->
    <div
      v-if="searchVisible"
      class="absolute top-2 right-2 bg-white border border-gray-300 rounded shadow-lg p-2 z-20 flex items-center gap-2 dark:bg-gray-800 dark:border-gray-600"
    >
      <input
        ref="searchInput"
        v-model="searchQuery"
        type="text"
        placeholder="Find..."
        class="bg-gray-100 border border-gray-300 rounded px-2 py-1 text-xs text-gray-900 w-40 focus:outline-none focus:border-blue-500 dark:bg-gray-700 dark:border-gray-600 dark:text-white"
        @keydown.enter="findNext"
        @keydown.shift.enter="findPrevious"
        @keydown.esc="closeSearch"
      />
      <button
        @click="findPrevious"
        class="text-gray-600 hover:text-gray-900 px-1 dark:text-gray-300 dark:hover:text-white"
        title="Previous"
      >
        ↑
      </button>
      <button
        @click="findNext"
        class="text-gray-600 hover:text-gray-900 px-1 dark:text-gray-300 dark:hover:text-white"
        title="Next"
      >
        ↓
      </button>
      <label class="flex items-center gap-1 text-xs text-gray-600 cursor-pointer select-none dark:text-gray-300">
        <input v-model="searchCaseSensitive" type="checkbox" class="accent-blue-500" />
        Aa
      </label>
      <button @click="closeSearch" class="text-gray-400 hover:text-gray-900 px-1 dark:hover:text-white">×</button>
    </div>

    <!-- Disconnect banner -->
    <div
      v-if="isDisconnected"
      class="absolute inset-0 bg-gray-100/90 flex flex-col items-center justify-center z-10 dark:bg-gray-900/90"
    >
      <p class="text-red-500 text-base font-semibold mb-3 dark:text-red-400">Connection lost</p>
      <button
        @click="reconnect"
        :disabled="isReconnecting"
        class="px-4 py-1.5 bg-blue-600 hover:bg-blue-700 disabled:bg-gray-300 text-white rounded text-xs font-medium dark:disabled:bg-gray-600"
      >
        {{ isReconnecting ? 'Reconnecting...' : 'Reconnect' }}
      </button>
    </div>

    <!-- Context menu -->
    <div
      v-if="contextMenu.show"
      ref="contextMenuEl"
      class="fixed bg-white border border-gray-300 rounded shadow-lg py-1 z-50 min-w-[8rem] dark:bg-gray-700 dark:border-gray-600"
      :style="{ top: contextMenu.y + 'px', left: contextMenu.x + 'px' }"
    >
      <button @click="copySelection" class="block w-full text-left px-3 py-1 text-xs text-gray-900 hover:bg-gray-100 dark:text-white dark:hover:bg-gray-600">Copy</button>
      <button @click="pasteFromClipboard" class="block w-full text-left px-3 py-1 text-xs text-gray-900 hover:bg-gray-100 dark:text-white dark:hover:bg-gray-600">Paste</button>
      <button @click="selectAll" class="block w-full text-left px-3 py-1 text-xs text-gray-900 hover:bg-gray-100 dark:text-white dark:hover:bg-gray-600">Select All</button>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, onUnmounted, onActivated, onDeactivated, watch, nextTick } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { SearchAddon } from '@xterm/addon-search'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { Cpu, MemoryStick, HardDrive, Clock, Monitor, ChevronUp, ChevronDown } from 'lucide-vue-next'
import '@xterm/xterm/css/xterm.css'

const props = defineProps({
  sessionId: {
    type: String,
    required: true,
  },
  hostId: {
    type: Number,
    default: null,
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
let statusInterval = null

const isDisconnected = ref(false)
const isReconnecting = ref(false)
const contextMenu = ref({ show: false, x: 0, y: 0 })
const terminalBgClass = ref('bg-gray-900')

const searchVisible = ref(false)
const searchQuery = ref('')
const searchCaseSensitive = ref(false)

const statusExpanded = ref(true)
const status = ref({ load: '', ram: '', disk: '', uptime: '', os: '', cores: '' })
const statusError = ref('')

const tooltip = ref({ show: false, text: '', x: 0, y: 0 })

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

function showTooltip(event, text) {
  tooltip.value = {
    show: true,
    text,
    x: event.clientX,
    y: event.clientY - 28,
  }
}

function hideTooltip() {
  tooltip.value.show = false
}

async function fetchSystemStatus() {
  if (!props.hostId || isDisconnected.value) return
  try {
    const [load, ram, disk, uptime, osName, kernel, arch, cores] = await Promise.all([
      invoke('ssh_exec', { hostId: props.hostId, command: "awk '{print $1}' /proc/loadavg" }).catch(() => ''),
      invoke('ssh_exec', { hostId: props.hostId, command: "free -m | awk 'NR==2{used=$3;total=$2;pct=used*100/total; if(total>=1024){printf \"%.1f/%.1fGB (%.0f%%)\", used/1024,total/1024,pct} else {printf \"%.0f/%.0fMB (%.0f%%)\", used,total,pct}}'" }).catch(() => ''),
      invoke('ssh_exec', { hostId: props.hostId, command: "df -h / | awk 'NR==2{print $3\"/\"$2\" (\"$5\")\"}'" }).catch(() => ''),
      invoke('ssh_exec', { hostId: props.hostId, command: "awk '{d=int($1/86400);h=int(($1%86400)/3600);m=int(($1%3600)/60); printf \"%dd %dh %dm\", d,h,m}' /proc/uptime" }).catch(() => ''),
      invoke('ssh_exec', { hostId: props.hostId, command: "grep '^PRETTY_NAME=' /etc/os-release | sed 's/PRETTY_NAME=//; s/\"//g'" }).catch(() => ''),
      invoke('ssh_exec', { hostId: props.hostId, command: 'uname -r' }).catch(() => ''),
      invoke('ssh_exec', { hostId: props.hostId, command: 'uname -m' }).catch(() => ''),
      invoke('ssh_exec', { hostId: props.hostId, command: 'nproc' }).catch(() => ''),
    ])
    const osParts = [osName, kernel, arch].filter(Boolean)
    status.value = {
      load,
      ram,
      disk,
      uptime,
      os: osParts.join(' · '),
      cores,
    }
    statusError.value = ''
  } catch (err) {
    statusError.value = ''
  }
}

function startStatusPolling() {
  if (statusInterval) clearInterval(statusInterval)
  if (!props.hostId) return
  fetchSystemStatus()
  statusInterval = setInterval(fetchSystemStatus, 5000)
}

function stopStatusPolling() {
  if (statusInterval) {
    clearInterval(statusInterval)
    statusInterval = null
  }
}

async function initTerminal() {
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
      startStatusPolling()
    }
  })

  // Listen for SSH disconnections
  unlistenDisconnected = await listen('ssh-disconnected', (event) => {
    if (event.payload === props.sessionId) {
      isDisconnected.value = true
      stopStatusPolling()
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
      startStatusPolling()
    }
  })

  // Send keystrokes to SSH
  term.onData((data) => {
    invoke('ssh_write', { sessionId: props.sessionId, data }).catch((err) => {
      console.error('ssh_write failed:', err)
    })
  })

  // Listen for settings changes
  window.addEventListener('terminal-settings-changed', onSettingsChanged)
  window.addEventListener('click', onWindowClick)
  window.addEventListener('contextmenu', onWindowContextMenu, true)
}

function startActiveOperations() {
  if (!term) return
  // Handle resize
  if (!resizeObserver) {
    resizeObserver = new ResizeObserver(() => {
      if (fitAddon) {
        fitAddon.fit()
      }
    })
  }
  if (terminalContainer.value) {
    resizeObserver.observe(terminalContainer.value)
  }
  // Fit on activation in case size changed while inactive
  if (fitAddon) {
    setTimeout(() => fitAddon.fit(), 50)
  }
  // Start status polling if already connected
  startStatusPolling()
}

function stopActiveOperations() {
  stopStatusPolling()
  if (resizeObserver) {
    resizeObserver.disconnect()
    resizeObserver = null
  }
}

function disposeTerminal() {
  stopActiveOperations()
  if (unlistenData) { unlistenData(); unlistenData = null }
  if (unlistenError) { unlistenError(); unlistenError = null }
  if (unlistenConnected) { unlistenConnected(); unlistenConnected = null }
  if (unlistenDisconnected) { unlistenDisconnected(); unlistenDisconnected = null }
  if (unlistenReconnected) { unlistenReconnected(); unlistenReconnected = null }
  if (term) {
    term.dispose()
    term = null
  }
  fitAddon = null
  searchAddon = null
  window.removeEventListener('terminal-settings-changed', onSettingsChanged)
  window.removeEventListener('click', onWindowClick)
  window.removeEventListener('contextmenu', onWindowContextMenu, true)
  if (terminalContainer.value) {
    terminalContainer.value.removeEventListener('contextmenu', showContextMenu)
  }
}

async function reconnect() {
  isReconnecting.value = true
  try {
    await invoke('ssh_reconnect', { sessionId: props.sessionId })
  } catch (err) {
    console.error('Reconnect failed:', err)
    isReconnecting.value = false
  }
}

onMounted(async () => {
  await initTerminal()
  startActiveOperations()
})

onActivated(() => {
  startActiveOperations()
})

onDeactivated(() => {
  stopActiveOperations()
})

onUnmounted(() => {
  disposeTerminal()
})

// Handle sessionId changes
watch(() => props.sessionId, (newId, oldId) => {
  if (term && newId !== oldId) {
    term.clear()
  }
})
</script>