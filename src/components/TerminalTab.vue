<template>
  <div class="relative w-full h-full flex flex-col">
    <div ref="terminalContainer" class="flex-1 min-h-0" :class="terminalBgClass"></div>

    <!-- Docker Bottom Pane -->
    <div
      v-if="dockerPane.show"
      class="shrink-0 border-t border-[#3c3c3c] flex flex-col bg-[#1e1e1e]"
      :style="{ height: dockerPane.height + 'px' }"
    >
      <!-- Resize handle -->
      <div
        class="h-1.5 cursor-row-resize bg-[#3c3c3c] hover:bg-[#007acc] transition-colors"
        @mousedown="startResizeDockerPane"
      ></div>
      <!-- Header -->
      <div class="flex items-center justify-between px-2 py-1 border-b border-[#3c3c3c] shrink-0">
        <span class="text-[10px] text-[#cccccc] flex items-center gap-1.5">
          <FileText v-if="dockerPane.type === 'logs'" :size="10" />
          <TerminalIcon v-else :size="10" />
          {{ dockerPane.title }}
        </span>
        <div class="flex items-center gap-1.5">
          <button
            v-if="dockerPane.type === 'logs'"
            @click="toggleFollow"
            class="text-[10px] px-2 py-0.5 rounded font-medium transition-colors"
            :class="dockerPane.following
              ? 'bg-[#89d185]/20 text-[#89d185] hover:bg-[#89d185]/30'
              : 'bg-[#3c3c3c] text-[#858585] hover:bg-[#4c4c4c] hover:text-[#cccccc]'"
          >
            {{ dockerPane.following ? '● Following' : 'Follow' }}
          </button>
          <button
            @click="closeDockerPane"
            class="text-[#858585] hover:text-[#cccccc] px-1 text-xs leading-none"
          >
            ×
          </button>
        </div>
      </div>
      <!-- Terminal container -->
      <div ref="dockerPaneContainer" class="flex-1 min-h-0"></div>
    </div>

    <!-- Expanded System Panel -->
    <div
      v-if="props.hostId && statusExpanded"
      class="shrink-0 border-t border-[#3c3c3c] bg-[#1e1e1e] h-48 flex flex-col"
    >
      <!-- Sub-tabs -->
      <div class="flex border-b border-[#3c3c3c] px-2">
        <button
          v-for="t in ['processes', 'network', 'disk']"
          :key="t"
          @click="sysTab = t"
          class="px-2 py-0.5 text-[10px] font-medium capitalize transition-colors"
          :class="sysTab === t ? 'text-[#007acc]' : 'text-[#858585] hover:text-[#cccccc]'"
        >
          {{ t }}
        </button>
      </div>

      <!-- Content -->
      <div class="flex-1 overflow-y-auto p-1">
        <div v-if="sysLoading" class="flex items-center justify-center h-full">
          <Loader2 :size="14" class="animate-spin text-[#858585]" />
        </div>

        <!-- Processes -->
        <div v-else-if="sysTab === 'processes'" class="text-[10px]">
          <div class="grid grid-cols-12 gap-1 text-[#6e6e6e] font-medium border-b border-[#3c3c3c] pb-0.5 mb-0.5">
            <span class="col-span-1">PID</span>
            <span class="col-span-5">Command</span>
            <span class="col-span-2 text-right">CPU</span>
            <span class="col-span-2 text-right">Mem</span>
            <span class="col-span-2 text-right">Time</span>
          </div>
          <div
            v-for="p in processes"
            :key="p.pid"
            class="grid grid-cols-12 gap-1 text-[#cccccc] hover:bg-[#2a2d2e] py-0.5"
          >
            <span class="col-span-1 font-mono">{{ p.pid }}</span>
            <span class="col-span-5 truncate">{{ p.command }}</span>
            <span class="col-span-2 text-right" :class="parseFloat(p.cpu) > 50 ? 'text-[#f44336]' : ''">{{ p.cpu }}%</span>
            <span class="col-span-2 text-right">{{ p.mem }}%</span>
            <span class="col-span-2 text-right text-[#858585]">{{ p.uptime }}</span>
          </div>
        </div>

        <!-- Network -->
        <div v-else-if="sysTab === 'network'" class="text-[10px]">
          <div class="mb-1">
            <span class="text-[#858585]">Established:</span>
            <span class="text-[#cccccc] ml-1">{{ network?.established_count || 0 }}</span>
          </div>
          <div v-if="visiblePorts.length" class="mb-1">
            <div class="text-[#6e6e6e] font-medium mb-0.5">Listening Ports</div>
            <div
              v-for="p in visiblePorts"
              :key="p.local"
              class="grid grid-cols-3 gap-1 text-[#cccccc] hover:bg-[#2a2d2e] py-0.5"
            >
              <span>{{ p.proto }}</span>
              <span class="truncate">{{ p.local }}</span>
              <span class="truncate text-[#858585]">{{ p.process }}</span>
            </div>
          </div>
          <div v-if="visibleInterfaces.length">
            <div class="text-[#6e6e6e] font-medium mb-0.5">Interfaces</div>
            <div
              v-for="iface in visibleInterfaces"
              :key="iface.name"
              class="grid grid-cols-4 gap-1 text-[#cccccc] py-0.5"
            >
              <span>{{ iface.name }}</span>
              <span class="text-[#858585]">RX: {{ formatBytes(iface.rx_bytes) }}</span>
              <span class="text-[#858585]">TX: {{ formatBytes(iface.tx_bytes) }}</span>
            </div>
          </div>
        </div>

        <!-- Disk -->
        <div v-else-if="sysTab === 'disk'" class="text-[10px]">
          <div v-if="diskInfo?.mounts?.length" class="mb-1">
            <div class="text-[#6e6e6e] font-medium mb-0.5">Filesystems</div>
            <div
              v-for="m in diskInfo.mounts"
              :key="m.mount"
              class="grid grid-cols-6 gap-1 text-[#cccccc] hover:bg-[#2a2d2e] py-0.5"
            >
              <span class="col-span-2 truncate">{{ m.mount }}</span>
              <span class="col-span-1">{{ m.size }}</span>
              <span class="col-span-1">{{ m.used }}</span>
              <span class="col-span-1" :class="parseInt(m.percent) > 80 ? 'text-[#f44336]' : parseInt(m.percent) > 60 ? 'text-[#cca700]' : 'text-[#89d185]'">{{ m.percent }}%</span>
              <span class="col-span-1 text-[#858585] truncate">{{ m.filesystem }}</span>
            </div>
          </div>
          <div v-if="diskInfo?.dirs?.length">
            <div class="text-[#6e6e6e] font-medium mb-0.5 mt-1">Top Directories</div>
            <div
              v-for="d in diskInfo.dirs"
              :key="d.path"
              class="grid grid-cols-2 gap-1 text-[#cccccc] hover:bg-[#2a2d2e] py-0.5"
            >
              <span class="truncate">{{ d.path }}</span>
              <span class="text-[#858585]">{{ d.size }}</span>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- System Status Bar -->
    <div
      v-if="props.hostId"
      class="shrink-0 border-t border-[#3c3c3c] bg-[#1e1e1e] cursor-pointer hover:bg-[#252526] transition-colors"
      @click="statusExpanded = !statusExpanded"
    >
      <div class="flex items-center justify-between px-2 py-0.5">
        <div class="flex items-center gap-3 overflow-x-auto">
          <!-- Disconnected -->
          <div v-if="isDisconnected" class="flex items-center gap-1 text-[10px] text-[#f44336] whitespace-nowrap">
            <span class="w-1.5 h-1.5 rounded-full bg-[#f44336] animate-pulse" />
            <span class="font-medium">Disconnected</span>
          </div>
          <!-- Loading -->
          <div v-else-if="statusLoading && !status.load" class="flex items-center gap-1 text-[10px] text-[#858585] whitespace-nowrap">
            <Loader2 :size="10" class="animate-spin" />
            <span>Loading stats…</span>
          </div>
          <!-- Stats -->
          <template v-else>
            <div
              v-if="status.os"
              class="flex items-center gap-1 text-[10px] text-[#858585] max-w-[140px] cursor-default"
              @mouseenter="showTooltip($event, status.os)"
              @mouseleave="hideTooltip"
            >
              <Monitor :size="10" class="text-[#4ec9b0] shrink-0" />
              <span class="truncate">{{ status.os }}</span>
            </div>
            <div v-if="status.load" class="flex items-center gap-1 text-[10px] text-[#858585] whitespace-nowrap">
              <Cpu :size="10" class="text-[#569cd6]" />
              <span class="font-medium">CPU:</span>
              <span>{{ status.load }}<span v-if="status.cores"> / {{ status.cores }} cores</span></span>
            </div>
            <div v-if="status.ram" class="flex items-center gap-1 text-[10px] text-[#858585] whitespace-nowrap">
              <MemoryStick :size="10" class="text-[#89d185]" />
              <span class="font-medium">RAM:</span>
              <span>{{ status.ram }}</span>
            </div>
            <div v-if="status.disk" class="flex items-center gap-1 text-[10px] text-[#858585] whitespace-nowrap">
              <HardDrive :size="10" class="text-[#cca700]" />
              <span class="font-medium">Disk:</span>
              <span>{{ status.disk }}</span>
            </div>
            <div v-if="status.uptime" class="flex items-center gap-1 text-[10px] text-[#858585] whitespace-nowrap">
              <Clock :size="10" class="text-[#c586c0]" />
              <span class="font-medium">Up:</span>
              <span>{{ status.uptime }}</span>
            </div>
            <div class="flex items-center gap-1 text-[10px] text-[#858585] whitespace-nowrap">
              <ArrowDown :size="10" class="text-[#89d185]" />
              <span>{{ status.netDown || '—' }}</span>
            </div>
            <div class="flex items-center gap-1 text-[10px] text-[#858585] whitespace-nowrap">
              <ArrowUp :size="10" class="text-[#569cd6]" />
              <span>{{ status.netUp || '—' }}</span>
            </div>
          </template>
          <!-- Error -->
          <div v-if="statusError" class="flex items-center gap-1 text-[10px] text-[#f44336] whitespace-nowrap" :title="statusError">
            <span class="w-1.5 h-1.5 rounded-full bg-[#f44336]" />
            <span class="truncate max-w-[200px]">{{ statusError }}</span>
          </div>
        </div>
        <button
          @click.stop="statusExpanded = !statusExpanded"
          class="text-[#858585] hover:text-[#cccccc] shrink-0 ml-2"
          :title="statusExpanded ? 'Hide system panel' : 'Show system panel'"
        >
          <ChevronUp v-if="statusExpanded" :size="12" />
          <ChevronDown v-else :size="12" />
        </button>
      </div>
    </div>

    <!-- Custom tooltip -->
    <div
      v-if="tooltip.show"
      class="fixed z-50 px-2 py-1 bg-[#252526] text-[#cccccc] text-xs rounded shadow-lg pointer-events-none whitespace-nowrap border border-[#3c3c3c]"
      :style="{ left: tooltip.x + 'px', top: tooltip.y + 'px' }"
    >
      {{ tooltip.text }}
    </div>

    <!-- Search bar -->
    <div
      v-if="searchVisible"
      class="absolute top-2 right-2 bg-[#252526] border border-[#3c3c3c] rounded shadow-lg p-2 z-20 flex items-center gap-2"
    >
      <input
        ref="searchInput"
        v-model="searchQuery"
        type="text"
        placeholder="Find..."
        class="bg-[#3c3c3c] border border-[#3c3c3c] rounded px-2 py-1 text-xs text-[#cccccc] w-40 focus:outline-none focus:border-[#007acc] placeholder-[#6e6e6e]"
        @keydown.enter="findNext"
        @keydown.shift.enter="findPrevious"
        @keydown.esc="closeSearch"
      />
      <button
        @click="findPrevious"
        class="text-[#858585] hover:text-[#cccccc] px-1"
        title="Previous"
      >
        ↑
      </button>
      <button
        @click="findNext"
        class="text-[#858585] hover:text-[#cccccc] px-1"
        title="Next"
      >
        ↓
      </button>
      <label class="flex items-center gap-1 text-xs text-[#858585] cursor-pointer select-none">
        <input v-model="searchCaseSensitive" type="checkbox" class="accent-[#007acc]" />
        Aa
      </label>
      <button @click="closeSearch" class="text-[#858585] hover:text-[#cccccc] px-1">×</button>
    </div>

    <!-- Disconnect banner -->
    <div
      v-if="isDisconnected"
      class="absolute inset-0 bg-[#1e1e1e]/90 flex flex-col items-center justify-center z-10"
    >
      <p class="text-[#f44336] text-base font-semibold mb-3">Connection lost</p>
      <button
        @click="reconnect"
        :disabled="isReconnecting"
        class="px-4 py-1.5 bg-[#0e639c] hover:bg-[#1177bb] disabled:bg-[#3c3c3c] text-[#cccccc] rounded text-xs font-medium"
      >
        {{ isReconnecting ? 'Reconnecting...' : 'Reconnect' }}
      </button>
    </div>

    <!-- Context menu -->
    <div
      v-if="contextMenu.show"
      ref="contextMenuEl"
      class="fixed bg-[#252526] border border-[#3c3c3c] rounded shadow-lg py-1 z-50 min-w-[8rem]"
      :style="{ top: contextMenu.y + 'px', left: contextMenu.x + 'px' }"
    >
      <button @click="copySelection" class="block w-full text-left px-3 py-1 text-xs text-[#cccccc] hover:bg-[#2a2d2e]">Copy</button>
      <button @click="pasteFromClipboard" class="block w-full text-left px-3 py-1 text-xs text-[#cccccc] hover:bg-[#2a2d2e]">Paste</button>
      <button @click="selectAll" class="block w-full text-left px-3 py-1 text-xs text-[#cccccc] hover:bg-[#2a2d2e]">Select All</button>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { SearchAddon } from '@xterm/addon-search'
import { WebglAddon } from '@xterm/addon-webgl'
import { listen } from '@tauri-apps/api/event'
import { invoke, Channel } from '@tauri-apps/api/core'
import { Cpu, MemoryStick, HardDrive, Clock, Monitor, ChevronUp, ChevronDown, Loader2, ArrowDown, ArrowUp, FileText, Terminal as TerminalIcon } from 'lucide-vue-next'
import { TERMINAL_THEME } from '../themes/index.js'
import { useConnectionStore } from '../stores/connection.js'
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
  isActive: {
    type: Boolean,
    default: false,
  },
})

const store = useConnectionStore()

const terminalContainer = ref(null)
const contextMenuEl = ref(null)
const searchInput = ref(null)
let term = null
let fitAddon = null
let searchAddon = null
let webglAddon = null
let resizeObserver = null
let statusInterval = null
let lazyDisposeTimer = null
let hasBeenInitialized = false

const isDisconnected = ref(false)

// Docker bottom pane
const dockerPane = ref({
  show: false,
  height: 200,
  ptySessionId: null,
  title: '',
  type: null,
  following: false,
})
const dockerPaneContainer = ref(null)
let dockerTerm = null
let dockerFitAddon = null
let dockerWebglAddon = null
let dockerPaneResizeObserver = null
let unlistenPtyData = null
let unlistenPtyError = null
let unlistenPtyConnected = null
let unlistenPtyDisconnected = null
let dockerKeyFlushTimer = null
const isReconnecting = ref(false)
const contextMenu = ref({ show: false, x: 0, y: 0 })
const terminalBgClass = ref('bg-gray-900')

const searchVisible = ref(false)
const searchQuery = ref('')
const searchCaseSensitive = ref(false)

const statusExpanded = ref(false)
const status = ref({ load: '', ram: '', disk: '', uptime: '', os: '', cores: '', netDown: '', netUp: '' })
const statusLoading = ref(false)
const statusError = ref('')

const sysTab = ref('processes')
const processes = ref([])
const network = ref(null)
const diskInfo = ref(null)
const sysLoading = ref(false)
let sysPollInterval = null

const visiblePorts = computed(() => network.value?.ports?.slice(0, 8) ?? [])
const visibleInterfaces = computed(() => network.value?.interfaces?.filter(i => i.name !== 'lo').slice(0, 4) ?? [])

const tooltip = ref({ show: false, text: '', x: 0, y: 0 })

function applySettings(settings) {
  if (!term) return
  if (settings.fontSize !== undefined) {
    term.options.fontSize = settings.fontSize
  }
  term.options.theme = TERMINAL_THEME
  terminalBgClass.value = 'bg-[#1e1e1e]'
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

// Docker pane methods
function startResizeDockerPane(e) {
  const startY = e.clientY
  const startHeight = dockerPane.value.height
  const onMove = (moveEvent) => {
    const delta = startY - moveEvent.clientY
    dockerPane.value.height = Math.max(80, Math.min(400, startHeight + delta))
  }
  const onUp = () => {
    document.removeEventListener('mousemove', onMove)
    document.removeEventListener('mouseup', onUp)
  }
  document.addEventListener('mousemove', onMove)
  document.addEventListener('mouseup', onUp)
}

async function openDockerPane({ type, containerId, containerName, command }) {
  // Close any existing pane first
  await closeDockerPane()

  const ptySessionId = crypto.randomUUID()
  dockerPane.value.show = true
  dockerPane.value.ptySessionId = ptySessionId
  dockerPane.value.type = type
  dockerPane.value.following = type === 'logs' && command.includes(' -f ')
  dockerPane.value.title = type === 'logs'
    ? `Logs: ${containerName}`
    : `Exec: ${containerName}`

  await nextTick()

  const fontSizeSetting = await invoke('get_setting', { key: 'font_size' })
  const fontSize = fontSizeSetting ? parseInt(fontSizeSetting) : 14

  dockerTerm = new Terminal({
    cursorBlink: true,
    fontSize,
    fontFamily: 'Menlo, Monaco, "Courier New", monospace',
    theme: TERMINAL_THEME,
  })

  dockerFitAddon = new FitAddon()
  dockerWebglAddon = new WebglAddon()
  dockerTerm.loadAddon(dockerFitAddon)
  dockerTerm.loadAddon(dockerWebglAddon)

  dockerTerm.open(dockerPaneContainer.value)
  dockerFitAddon.fit()

  // Batch keystrokes
  let dockerKeyBuffer = ''
  function flushDockerKeyBuffer() {
    dockerKeyFlushTimer = null
    if (!dockerPane.value.show || dockerPane.value.ptySessionId !== ptySessionId) {
      dockerKeyBuffer = ''
      return
    }
    if (dockerKeyBuffer) {
      invoke('exec_pty_write', { ptySessionId, data: dockerKeyBuffer }).catch((err) => {
        // Ignore "not found" errors — session may have closed naturally
        if (!String(err).includes('not found')) {
          console.error('exec_pty_write failed:', err)
        }
      })
      dockerKeyBuffer = ''
    }
  }

  dockerTerm.onData((data) => {
    dockerKeyBuffer += data
    if (!dockerKeyFlushTimer) {
      dockerKeyFlushTimer = setTimeout(flushDockerKeyBuffer, 16)
    }
  })

  // Listen for PTY data
  unlistenPtyData = await listen('exec-pty-data', (event) => {
    const payload = event.payload
    if (typeof payload === 'object' && payload.pty_session_id === ptySessionId) {
      dockerTerm.write(payload.data)
    }
  })

  unlistenPtyError = await listen('exec-pty-error', (event) => {
    const payload = event.payload
    if (typeof payload === 'object' && payload.pty_session_id === ptySessionId) {
      dockerTerm.writeln(`\r\n\x1b[31mError: ${payload.error}\x1b[0m`)
    }
  })

  unlistenPtyConnected = await listen('exec-pty-connected', (event) => {
    if (event.payload === ptySessionId) {
      setTimeout(() => {
        if (dockerFitAddon) dockerFitAddon.fit()
      }, 100)
    }
  })

  unlistenPtyDisconnected = await listen('exec-pty-disconnected', (event) => {
    if (event.payload === ptySessionId) {
      dockerPane.value.following = false
      // Auto-close exec panes when the shell exits
      if (dockerPane.value.type === 'exec') {
        closeDockerPane()
      }
    }
  })

  // Observe resize
  if (!dockerPaneResizeObserver) {
    dockerPaneResizeObserver = new ResizeObserver(() => {
      if (dockerFitAddon) {
        dockerFitAddon.fit()
      }
    })
  }
  if (dockerPaneContainer.value) {
    dockerPaneResizeObserver.observe(dockerPaneContainer.value)
  }

  // Binary data channel for Docker exec PTY
  const dockerDataChannel = new Channel()
  dockerDataChannel.onmessage = (message) => {
    if (message instanceof Uint8Array) {
      dockerTerm.write(message)
    } else if (Array.isArray(message)) {
      dockerTerm.write(new Uint8Array(message))
    } else if (typeof message === 'string') {
      dockerTerm.write(message)
    }
  }
  invoke('open_exec_pty_data_channel', { ptySessionId, channel: dockerDataChannel }).catch(() => {})

  // Start the PTY session
  try {
    await invoke('exec_pty_connect', { hostId: props.hostId, ptySessionId, command })
  } catch (err) {
    console.error('exec_pty_connect failed:', err)
    dockerTerm.writeln(`\r\n\x1b[31mFailed to start: ${err}\x1b[0m`)
  }
}

async function closeDockerPane() {
  if (dockerKeyFlushTimer) {
    clearTimeout(dockerKeyFlushTimer)
    dockerKeyFlushTimer = null
  }

  if (dockerPane.value.ptySessionId) {
    await invoke('exec_pty_disconnect', { ptySessionId: dockerPane.value.ptySessionId }).catch(() => {})
  }

  if (dockerPaneResizeObserver) {
    dockerPaneResizeObserver.disconnect()
    dockerPaneResizeObserver = null
  }

  if (unlistenPtyData) { unlistenPtyData(); unlistenPtyData = null }
  if (unlistenPtyError) { unlistenPtyError(); unlistenPtyError = null }
  if (unlistenPtyConnected) { unlistenPtyConnected(); unlistenPtyConnected = null }
  if (unlistenPtyDisconnected) { unlistenPtyDisconnected(); unlistenPtyDisconnected = null }

  if (dockerTerm) {
    dockerTerm.dispose()
    dockerTerm = null
  }
  dockerFitAddon = null
  dockerWebglAddon = null

  dockerPane.value.show = false
  dockerPane.value.ptySessionId = null
  dockerPane.value.title = ''
  dockerPane.value.type = null
  dockerPane.value.following = false
}

function shellEscape(s) {
  if (!s) return "''"
  if (/^[a-zA-Z0-9._~\-\/:@]+$/.test(s)) return s
  return "'" + s.replace(/'/g, "'\"'\"'") + "'"
}

async function toggleFollow() {
  if (!dockerPane.value.show || dockerPane.value.type !== 'logs') return
  const following = dockerPane.value.following
  const containerName = dockerPane.value.title.replace('Logs: ', '')
  const escapedName = shellEscape(containerName)
  const cmd = following
    ? `docker logs --tail 200 ${escapedName}`
    : `docker logs -f --tail 200 ${escapedName}`
  await openDockerPane({ type: 'logs', containerId: '', containerName, command: cmd })
}

function onDockerPaneOpen(event) {
  if (event.detail.sessionId === props.sessionId) {
    openDockerPane(event.detail)
  }
}

function formatBytes(bytes) {
  if (bytes < 1024) return bytes + ' B'
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB'
  if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + ' MB'
  return (bytes / (1024 * 1024 * 1024)).toFixed(1) + ' GB'
}

function formatRate(bytesPerSec) {
  const abs = Math.abs(bytesPerSec)
  if (abs < 1024) return bytesPerSec.toFixed(0) + ' B/s'
  if (abs < 1024 * 1024) return (bytesPerSec / 1024).toFixed(1) + ' KB/s'
  return (bytesPerSec / (1024 * 1024)).toFixed(1) + ' MB/s'
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
  statusLoading.value = true
  try {
    const result = await invoke('get_system_stats', { hostId: props.hostId })

    // Compute network rates
    let netDown = ''
    let netUp = ''
    if (result.netdev) {
      let rxTotal = 0
      let txTotal = 0
      for (const line of result.netdev.split('\n')) {
        const parts = line.trim().split(/\s+/)
        if (parts.length >= 3) {
          const iface = parts[0].replace(':', '')
          if (iface === 'lo') continue
          const rx = parseInt(parts[1]) || 0
          const tx = parseInt(parts[2]) || 0
          rxTotal += rx
          txTotal += tx
        }
      }
      const now = Date.now()
      const prev = store.getNetStats(props.hostId)
      if (prev.time > 0 && prev.rx > 0 && prev.tx > 0) {
        const elapsed = (now - prev.time) / 1000
        if (elapsed > 0) {
          const rxRate = (rxTotal - prev.rx) / elapsed
          const txRate = (txTotal - prev.tx) / elapsed
          netDown = formatRate(rxRate)
          netUp = formatRate(txRate)
        }
      } else {
        // First fetch: show cumulative totals instead of dash
        netDown = formatBytes(rxTotal)
        netUp = formatBytes(txTotal)
      }
      store.setNetStats(props.hostId, { rx: rxTotal, tx: txTotal, time: now })
    }

    const osParts = [result.os, result.kernel, result.arch].filter(Boolean)
    const data = {
      load: result.load || '',
      ram: result.ram || '',
      disk: result.disk || '',
      uptime: result.uptime || '',
      os: osParts.join(' · '),
      cores: result.cores || '',
      netDown,
      netUp,
    }
    status.value = data
    store.setSystemStatus(props.hostId, data)
    statusError.value = ''
  } catch (err) {
    console.warn('get_system_stats failed:', err)
    statusError.value = String(err).replace(/^Error: /, '')
  } finally {
    statusLoading.value = false
  }
}

function startStatusPolling() {
  if (statusInterval) clearInterval(statusInterval)
  if (!props.hostId) return
  // Don't poll when page is hidden
  if (document.hidden) return
  // Read from cache immediately
  const cached = store.getSystemStatus(props.hostId)
  if (cached) {
    status.value = cached
  }
  // Fetch immediately, then every 5s
  fetchSystemStatus()
  statusInterval = setInterval(fetchSystemStatus, 5000)
}

function stopStatusPolling() {
  if (statusInterval) {
    clearInterval(statusInterval)
    statusInterval = null
  }
}

function onVisibilityChange() {
  if (document.hidden) {
    stopStatusPolling()
    stopSysPolling()
  } else if (props.isActive) {
    startStatusPolling()
    if (statusExpanded.value) {
      startSysPolling()
    }
  }
}

async function fetchPanelData(includeDisk = false) {
  if (!props.hostId) return
  try {
    const panel = await invoke('get_system_panel', { hostId: props.hostId })
    processes.value = panel.processes || []
    network.value = panel.network || null
    if (includeDisk) {
      diskInfo.value = panel.disk || null
    }
  } catch (err) {
    console.error('get_system_panel failed:', err)
  }
}

async function loadSystemData() {
  if (!props.hostId) return
  sysLoading.value = true
  await fetchPanelData(true)
  sysLoading.value = false
}

function startSysPolling() {
  if (sysPollInterval) clearInterval(sysPollInterval)
  if (!props.hostId) return
  loadSystemData()
  sysPollInterval = setInterval(() => {
    fetchPanelData(false)
  }, 3000)
}

function stopSysPolling() {
  if (sysPollInterval) {
    clearInterval(sysPollInterval)
    sysPollInterval = null
  }
}

async function initTerminal() {
  const fontSizeSetting = await invoke('get_setting', { key: 'font_size' })
  const fontSize = fontSizeSetting ? parseInt(fontSizeSetting) : 14
  const themeSetting = await invoke('get_setting', { key: 'theme' })
  terminalBgClass.value = 'bg-[#1e1e1e]'

  term = new Terminal({
    cursorBlink: true,
    fontSize,
    fontFamily: 'Menlo, Monaco, "Courier New", monospace',
    theme: TERMINAL_THEME,
  })

  fitAddon = new FitAddon()
  searchAddon = new SearchAddon()
  webglAddon = new WebglAddon()
  webglAddon.onContextLoss(() => {
    // WebGL context lost (e.g., tab hidden). Re-add addon and refresh.
    if (term && webglAddon) {
      term.loadAddon(webglAddon)
      term.refresh(0, term.rows - 1)
    }
  })
  term.loadAddon(fitAddon)
  term.loadAddon(searchAddon)
  term.loadAddon(webglAddon)

  term.open(terminalContainer.value)
  fitAddon.fit()

  // Fit after flex layout settles
  requestAnimationFrame(() => {
    if (fitAddon) fitAddon.fit()
  })

  // Custom keyboard shortcuts
  term.attachCustomKeyEventHandler((e) => {
    if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === 'c') {
      copySelection()
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

  // Register with global event router in store (replaces per-tab listeners)
  store.registerTerminal(props.sessionId, {
    write: (data) => term.write(data),
    writeError: (error) => term.writeln(`\r\n\x1b[31mError: ${error}\x1b[0m`),
    onConnected: () => {
      setTimeout(() => {
        if (fitAddon) {
          fitAddon.fit()
          // Send exact size to remote so shell matches the real terminal dimensions
          const { cols, rows } = term
          if (cols > 0 && rows > 0) {
            invoke('ssh_resize', { sessionId: props.sessionId, cols, rows }).catch(() => {})
          }
        }
      }, 100)
      startStatusPolling()
    },
    onDisconnected: () => {
      isDisconnected.value = true
      stopStatusPolling()
      status.value = { load: '', ram: '', disk: '', uptime: '', os: '', cores: '', netDown: '', netUp: '' }
      statusError.value = ''
      closeDockerPane()
    },
    onReconnected: () => {
      isDisconnected.value = false
      isReconnecting.value = false
      term.clear()
      setTimeout(() => {
        if (fitAddon) {
          fitAddon.fit()
          const { cols, rows } = term
          if (cols > 0 && rows > 0) {
            invoke('ssh_resize', { sessionId: props.sessionId, cols, rows }).catch(() => {})
          }
        }
      }, 100)
      startStatusPolling()
    },
  })

  // Binary data channel for raw SSH output (bypasses JSON events)
  const dataChannel = new Channel()
  dataChannel.onmessage = (message) => {
    // Handle various possible data formats from Tauri Channel
    if (message instanceof Uint8Array) {
      term.write(message)
    } else if (Array.isArray(message)) {
      term.write(new Uint8Array(message))
    } else if (typeof message === 'string') {
      term.write(message)
    }
  }
  invoke('open_data_channel', { sessionId: props.sessionId, channel: dataChannel }).catch(() => {})

  // Smart input buffer: immediate for typing, chunked for paste
  let inputBuffer = ''
  let inputFlushTimer = null
  const INPUT_FLUSH_DELAY = 2
  const PASTE_CHUNK_SIZE = 512
  const PASTE_CHUNK_DELAY = 1

  function flushInputBuffer() {
    inputFlushTimer = null
    if (!inputBuffer) return
    const data = inputBuffer
    inputBuffer = ''
    invoke('ssh_write', { sessionId: props.sessionId, data }).catch(() => {})
  }

  function sendPasteChunks(data) {
    let offset = 0
    function sendNext() {
      if (offset >= data.length) return
      const chunk = data.slice(offset, offset + PASTE_CHUNK_SIZE)
      offset += PASTE_CHUNK_SIZE
      invoke('ssh_write', { sessionId: props.sessionId, data: chunk }).catch(() => {})
      if (offset < data.length) {
        setTimeout(sendNext, PASTE_CHUNK_DELAY)
      }
    }
    sendNext()
  }

  term.onData((data) => {
    if (data.length === 1) {
      // Single char: send immediately (typing)
      if (inputBuffer) flushInputBuffer()
      invoke('ssh_write', { sessionId: props.sessionId, data }).catch(() => {})
    } else if (data.length > 50) {
      // Large paste: chunk into pieces
      if (inputBuffer) flushInputBuffer()
      sendPasteChunks(data)
    } else {
      // Rapid typing: accumulate and flush on idle
      inputBuffer += data
      if (inputFlushTimer) clearTimeout(inputFlushTimer)
      inputFlushTimer = setTimeout(flushInputBuffer, INPUT_FLUSH_DELAY)
    }
  })

  // Notify remote shell when terminal size changes
  term.onResize(({ cols, rows }) => {
    if (cols <= 0 || rows <= 0) return
    invoke('ssh_resize', { sessionId: props.sessionId, cols, rows }).catch(() => {})
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
}

function stopActiveOperations() {
  stopStatusPolling()
  if (resizeObserver) {
    resizeObserver.disconnect()
    resizeObserver = null
  }
}

function disposeTerminal() {
  if (lazyDisposeTimer) {
    clearTimeout(lazyDisposeTimer)
    lazyDisposeTimer = null
  }
  stopActiveOperations()
  stopSysPolling()
  store.unregisterTerminal(props.sessionId)
  if (term) {
    term.dispose()
    term = null
  }
  fitAddon = null
  searchAddon = null
  webglAddon = null
  window.removeEventListener('terminal-settings-changed', onSettingsChanged)
  window.removeEventListener('click', onWindowClick)
  window.removeEventListener('contextmenu', onWindowContextMenu, true)
  if (terminalContainer.value) {
    terminalContainer.value.removeEventListener('contextmenu', showContextMenu)
  }
  closeDockerPane()
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
  hasBeenInitialized = true
  startActiveOperations()
  // Fix race condition: if tab is already active when terminal finishes init,
  // the isActive watcher already fired early (term was null). Start polling now.
  if (props.isActive) {
    startStatusPolling()
  }
  document.addEventListener('visibilitychange', onVisibilityChange)
  window.addEventListener('docker-pane-open', onDockerPaneOpen)
})

onUnmounted(() => {
  document.removeEventListener('visibilitychange', onVisibilityChange)
  window.removeEventListener('docker-pane-open', onDockerPaneOpen)
  disposeTerminal()
})

// Handle sessionId changes
watch(() => props.sessionId, (newId, oldId) => {
  if (term && newId !== oldId) {
    term.clear()
  }
})

// Handle active state changes (tab switching)
watch(() => props.isActive, (active) => {
  if (active) {
    // Cancel lazy dispose
    if (lazyDisposeTimer) {
      clearTimeout(lazyDisposeTimer)
      lazyDisposeTimer = null
    }
    // Recreate terminal if it was lazily disposed
    if (!term && hasBeenInitialized) {
      initTerminal().then(() => {
        startActiveOperations()
        if (props.isActive) {
          startStatusPolling()
        }
      })
      return
    }
    if (!term) return
    term.focus()
    nextTick(() => {
      requestAnimationFrame(() => {
        if (fitAddon) fitAddon.fit()
        if (dockerFitAddon) dockerFitAddon.fit()
        // Recreate WebGL addon after tab was hidden — browser may have dropped the context
        if (term && webglAddon) {
          try {
            webglAddon.dispose()
            webglAddon = new WebglAddon()
            webglAddon.onContextLoss(() => {
              if (term && webglAddon) {
                webglAddon.dispose()
                webglAddon = new WebglAddon()
                term.loadAddon(webglAddon)
                term.refresh(0, term.rows - 1)
              }
            })
            term.loadAddon(webglAddon)
          } catch (_) {}
        }
        if (term) term.refresh(0, term.rows - 1)
        if (dockerTerm) dockerTerm.refresh(0, dockerTerm.rows - 1)
      })
    })
    if (terminalContainer.value && resizeObserver) {
      resizeObserver.observe(terminalContainer.value)
    }
    startStatusPolling()
  } else {
    if (term) term.blur()
    stopStatusPolling()
    if (resizeObserver) {
      resizeObserver.disconnect()
    }
    // Start lazy dispose timer (30s)
    if (!lazyDisposeTimer && term) {
      lazyDisposeTimer = setTimeout(() => {
        lazyDisposeTimer = null
        disposeTerminal()
      }, 30000)
    }
  }
}, { immediate: true })

watch(statusExpanded, (expanded) => {
  if (expanded && props.isActive) {
    startSysPolling()
  } else {
    stopSysPolling()
  }
})
</script>