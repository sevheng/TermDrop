<template>
  <div class="relative w-full h-full flex flex-col">
    <div ref="terminalContainer" class="flex-1 min-h-0" :class="terminalBgClass"></div>

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
          <div v-if="network?.ports?.length" class="mb-1">
            <div class="text-[#6e6e6e] font-medium mb-0.5">Listening Ports</div>
            <div
              v-for="p in network.ports.slice(0, 8)"
              :key="p.local"
              class="grid grid-cols-3 gap-1 text-[#cccccc] hover:bg-[#2a2d2e] py-0.5"
            >
              <span>{{ p.proto }}</span>
              <span class="truncate">{{ p.local }}</span>
              <span class="truncate text-[#858585]">{{ p.process }}</span>
            </div>
          </div>
          <div v-if="network?.interfaces?.length">
            <div class="text-[#6e6e6e] font-medium mb-0.5">Interfaces</div>
            <div
              v-for="iface in network.interfaces.filter(i => i.name !== 'lo').slice(0, 4)"
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
          <div v-if="status.netDown" class="flex items-center gap-1 text-[10px] text-[#858585] whitespace-nowrap">
            <ArrowDown :size="10" class="text-[#89d185]" />
            <span>{{ status.netDown }}</span>
          </div>
          <div v-if="status.netUp" class="flex items-center gap-1 text-[10px] text-[#858585] whitespace-nowrap">
            <ArrowUp :size="10" class="text-[#569cd6]" />
            <span>{{ status.netUp }}</span>
          </div>
          <span v-if="statusError" class="text-[10px] text-[#f44336]">{{ statusError }}</span>
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
import { ref, onMounted, onUnmounted, watch, nextTick } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { SearchAddon } from '@xterm/addon-search'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { Cpu, MemoryStick, HardDrive, Clock, Monitor, ChevronUp, ChevronDown, Loader2, ArrowDown, ArrowUp } from 'lucide-vue-next'
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

const statusExpanded = ref(false)
const status = ref({ load: '', ram: '', disk: '', uptime: '', os: '', cores: '', netDown: '', netUp: '' })
const prevNetStats = ref({ rx: 0, tx: 0, time: 0 })
const statusError = ref('')

const sysTab = ref('processes')
const processes = ref([])
const network = ref(null)
const diskInfo = ref(null)
const sysLoading = ref(false)
let sysPollInterval = null

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
  try {
    const [load, ram, disk, uptime, osName, kernel, arch, cores, netDev] = await Promise.all([
      invoke('ssh_exec', { hostId: props.hostId, command: "awk '{print $1}' /proc/loadavg" }).catch(() => ''),
      invoke('ssh_exec', { hostId: props.hostId, command: "free -m | awk 'NR==2{used=$3;total=$2;pct=used*100/total; if(total>=1024){printf \"%.1f/%.1fGB (%.0f%%)\", used/1024,total/1024,pct} else {printf \"%.0f/%.0fMB (%.0f%%)\", used,total,pct}}'" }).catch(() => ''),
      invoke('ssh_exec', { hostId: props.hostId, command: "df -h / | awk 'NR==2{print $3\"/\"$2\" (\"$5\")\"}'" }).catch(() => ''),
      invoke('ssh_exec', { hostId: props.hostId, command: "awk '{d=int($1/86400);h=int(($1%86400)/3600);m=int(($1%3600)/60); printf \"%dd %dh %dm\", d,h,m}' /proc/uptime" }).catch(() => ''),
      invoke('ssh_exec', { hostId: props.hostId, command: "grep '^PRETTY_NAME=' /etc/os-release | sed 's/PRETTY_NAME=//; s/\"//g'" }).catch(() => ''),
      invoke('ssh_exec', { hostId: props.hostId, command: 'uname -r' }).catch(() => ''),
      invoke('ssh_exec', { hostId: props.hostId, command: 'uname -m' }).catch(() => ''),
      invoke('ssh_exec', { hostId: props.hostId, command: 'nproc' }).catch(() => ''),
      invoke('ssh_exec', { hostId: props.hostId, command: "cat /proc/net/dev | tail -n +3 | awk '{print $1\" \"$2\" \"$10}'" }).catch(() => ''),
    ])

    // Compute network rates
    let netDown = ''
    let netUp = ''
    if (netDev) {
      let rxTotal = 0
      let txTotal = 0
      for (const line of netDev.split('\n')) {
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
      const prev = prevNetStats.value
      if (prev.time > 0 && prev.rx > 0 && prev.tx > 0) {
        const elapsed = (now - prev.time) / 1000
        if (elapsed > 0) {
          const rxRate = (rxTotal - prev.rx) / elapsed
          const txRate = (txTotal - prev.tx) / elapsed
          netDown = formatRate(rxRate)
          netUp = formatRate(txRate)
        }
      }
      prevNetStats.value = { rx: rxTotal, tx: txTotal, time: now }
    }

    const osParts = [osName, kernel, arch].filter(Boolean)
    const data = {
      load,
      ram,
      disk,
      uptime,
      os: osParts.join(' · '),
      cores,
      netDown,
      netUp,
    }
    status.value = data
    store.setSystemStatus(props.hostId, data)
    statusError.value = ''
  } catch (err) {
    statusError.value = ''
  }
}

function startStatusPolling() {
  if (statusInterval) clearInterval(statusInterval)
  if (!props.hostId) return
  // Read from cache immediately instead of firing SSH execs
  const cached = store.getSystemStatus(props.hostId)
  if (cached) {
    status.value = cached
  }
  statusInterval = setInterval(fetchSystemStatus, 5000)
}

function stopStatusPolling() {
  if (statusInterval) {
    clearInterval(statusInterval)
    statusInterval = null
  }
}

async function fetchProcesses() {
  if (!props.hostId) return
  try {
    processes.value = await invoke('get_processes', { hostId: props.hostId })
  } catch (err) {
    console.error('get_processes failed:', err)
  }
}

async function fetchNetwork() {
  if (!props.hostId) return
  try {
    network.value = await invoke('get_network', { hostId: props.hostId })
  } catch (err) {
    console.error('get_network failed:', err)
  }
}

async function fetchDisk() {
  if (!props.hostId) return
  try {
    diskInfo.value = await invoke('get_disk_usage', { hostId: props.hostId })
  } catch (err) {
    console.error('get_disk_usage failed:', err)
  }
}

async function loadSystemData() {
  if (!props.hostId) return
  sysLoading.value = true
  await Promise.all([fetchProcesses(), fetchNetwork(), fetchDisk()])
  sysLoading.value = false
}

function startSysPolling() {
  if (sysPollInterval) clearInterval(sysPollInterval)
  if (!props.hostId) return
  loadSystemData()
  sysPollInterval = setInterval(() => {
    fetchProcesses()
    fetchNetwork()
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
  term.loadAddon(fitAddon)
  term.loadAddon(searchAddon)
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
  stopSysPolling()
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

onUnmounted(() => {
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
  if (!term) return
  if (active) {
    term.focus()
    requestAnimationFrame(() => {
      if (fitAddon) fitAddon.fit()
    })
    if (terminalContainer.value && resizeObserver) {
      resizeObserver.observe(terminalContainer.value)
    }
    startStatusPolling()
  } else {
    term.blur()
    stopStatusPolling()
    if (resizeObserver) {
      resizeObserver.disconnect()
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