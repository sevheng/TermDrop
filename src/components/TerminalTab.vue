<template>
  <div class="relative w-full h-full flex flex-col">
    <div ref="terminalContainer" class="flex-1 min-h-0" :class="terminalBgClass"></div>

    <!-- Docker Bottom Pane -->
    <div
      v-if="dockerPane.show"
      class="shrink-0 border-t border-line flex flex-col bg-canvas"
      :style="{ height: dockerPane.height + 'px' }"
    >
      <!-- Resize handle -->
      <div
        class="h-1.5 cursor-row-resize bg-input hover:bg-accent transition-colors"
        @mousedown="startResizeDockerPane"
      ></div>
      <!-- Header -->
      <div class="flex items-center justify-between px-2 py-1 border-b border-line shrink-0">
        <span class="text-2xs text-ink flex items-center gap-1.5">
          <FileText v-if="dockerPane.type === 'logs'" :size="10" />
          <TerminalIcon v-else :size="10" />
          {{ dockerPane.title }}
        </span>
        <div class="flex items-center gap-1.5">
          <button
            v-if="dockerPane.type === 'logs'"
            @click="toggleFollow"
            class="text-2xs px-2 py-0.5 rounded font-medium transition-colors"
            :class="dockerPane.following
              ? 'bg-good/20 text-good hover:bg-good/30'
              : 'bg-input text-ink-2 hover:bg-input-hover hover:text-ink'"
          >
            {{ dockerPane.following ? '● Following' : 'Follow' }}
          </button>
          <button
            @click="closeDockerPane"
            class="text-ink-2 hover:text-ink px-1 text-xs leading-none"
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
      class="shrink-0 border-t border-line bg-canvas h-48 flex flex-col"
    >
      <!-- Sub-tabs -->
      <div class="flex border-b border-line px-2">
        <button
          v-for="t in ['processes', 'network', 'disk']"
          :key="t"
          @click="sysTab = t"
          class="px-2 py-0.5 text-2xs font-medium capitalize transition-colors"
          :class="sysTab === t ? 'text-accent' : 'text-ink-2 hover:text-ink'"
        >
          {{ t }}
        </button>
      </div>

      <!-- Content -->
      <div class="flex-1 overflow-y-auto p-1">
        <div v-if="sysLoading" class="flex items-center justify-center h-full">
          <Loader2 :size="14" class="animate-spin text-ink-2" />
        </div>

        <!-- Processes -->
        <div v-else-if="sysTab === 'processes'" class="text-2xs">
          <div class="grid grid-cols-12 gap-1 text-ink-3 font-medium border-b border-line pb-0.5 mb-0.5">
            <span class="col-span-1">PID</span>
            <span class="col-span-5">Command</span>
            <span class="col-span-2 text-right">CPU</span>
            <span class="col-span-2 text-right">Mem</span>
            <span class="col-span-2 text-right">Time</span>
          </div>
          <div
            v-for="p in processes"
            :key="p.pid"
            class="grid grid-cols-12 gap-1 text-ink hover:bg-raised py-0.5"
          >
            <span class="col-span-1 font-mono">{{ p.pid }}</span>
            <span class="col-span-5 truncate">{{ p.command }}</span>
            <span class="col-span-2 text-right" :class="parseFloat(p.cpu) > 50 ? 'text-bad' : ''">{{ p.cpu }}%</span>
            <span class="col-span-2 text-right">{{ p.mem }}%</span>
            <span class="col-span-2 text-right text-ink-2">{{ p.uptime }}</span>
          </div>
        </div>

        <!-- Network -->
        <div v-else-if="sysTab === 'network'" class="text-2xs">
          <div class="mb-1">
            <span class="text-ink-2">Established:</span>
            <span class="text-ink ml-1">{{ network?.established_count || 0 }}</span>
          </div>
          <div v-if="visiblePorts.length" class="mb-1">
            <div class="text-ink-3 font-medium mb-0.5">Listening Ports</div>
            <div
              v-for="p in visiblePorts"
              :key="p.local"
              class="grid grid-cols-3 gap-1 text-ink hover:bg-raised py-0.5"
            >
              <span>{{ p.proto }}</span>
              <span class="truncate">{{ p.local }}</span>
              <span class="truncate text-ink-2">{{ p.process }}</span>
            </div>
          </div>
          <div v-if="visibleInterfaces.length">
            <div class="text-ink-3 font-medium mb-0.5">Interfaces</div>
            <div
              v-for="iface in visibleInterfaces"
              :key="iface.name"
              class="grid grid-cols-4 gap-1 text-ink py-0.5"
            >
              <span>{{ iface.name }}</span>
              <span class="text-ink-2">RX: {{ formatBytes(iface.rx_bytes) }}</span>
              <span class="text-ink-2">TX: {{ formatBytes(iface.tx_bytes) }}</span>
            </div>
          </div>
        </div>

        <!-- Disk -->
        <div v-else-if="sysTab === 'disk'" class="text-2xs">
          <div v-if="diskInfo?.mounts?.length" class="mb-1">
            <div class="text-ink-3 font-medium mb-0.5">Filesystems</div>
            <div
              v-for="m in diskInfo.mounts"
              :key="m.mount"
              class="grid grid-cols-6 gap-1 text-ink hover:bg-raised py-0.5"
            >
              <span class="col-span-2 truncate">{{ m.mount }}</span>
              <span class="col-span-1">{{ m.size }}</span>
              <span class="col-span-1">{{ m.used }}</span>
              <span class="col-span-1" :class="parseInt(m.percent) > 80 ? 'text-bad' : parseInt(m.percent) > 60 ? 'text-warn' : 'text-good'">{{ m.percent }}%</span>
              <span class="col-span-1 text-ink-2 truncate">{{ m.filesystem }}</span>
            </div>
          </div>
          <div v-if="diskInfo?.dirs?.length">
            <div class="text-ink-3 font-medium mb-0.5 mt-1">Top Directories</div>
            <div
              v-for="d in diskInfo.dirs"
              :key="d.path"
              class="grid grid-cols-2 gap-1 text-ink hover:bg-raised py-0.5"
            >
              <span class="truncate">{{ d.path }}</span>
              <span class="text-ink-2">{{ d.size }}</span>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- System Status Bar -->
    <div
      v-if="props.hostId"
      class="shrink-0 border-t border-line bg-canvas cursor-pointer hover:bg-surface transition-colors"
      @click="statusExpanded = !statusExpanded"
    >
      <div class="flex items-center justify-between px-2 py-0.5">
        <div class="flex items-center gap-3 overflow-x-auto">
          <!-- Disconnected -->
          <div v-if="isDisconnected" class="flex items-center gap-1 text-2xs text-bad whitespace-nowrap">
            <span class="w-1.5 h-1.5 rounded-full bg-bad animate-pulse" />
            <span class="font-medium">Disconnected</span>
          </div>
          <!-- Loading -->
          <div v-else-if="statusLoading && !status.load" class="flex items-center gap-1 text-2xs text-ink-2 whitespace-nowrap">
            <Loader2 :size="10" class="animate-spin" />
            <span>Loading stats…</span>
          </div>
          <!-- Stats -->
          <template v-else>
            <div
              v-if="status.os"
              class="flex items-center gap-1 text-2xs text-ink-2 max-w-[140px] cursor-default"
              @mouseenter="showTooltip($event, status.os)"
              @mouseleave="hideTooltip"
            >
              <Monitor :size="10" class="text-syn-teal shrink-0" />
              <span class="truncate">{{ status.os }}</span>
            </div>
            <div v-if="status.load" class="flex items-center gap-1 text-2xs text-ink-2 whitespace-nowrap">
              <Cpu :size="10" class="text-syn-blue" />
              <span class="font-medium">CPU:</span>
              <span>{{ status.load }}<span v-if="status.cores"> / {{ status.cores }} cores</span></span>
            </div>
            <div v-if="status.ram" class="flex items-center gap-1 text-2xs text-ink-2 whitespace-nowrap">
              <MemoryStick :size="10" class="text-good" />
              <span class="font-medium">RAM:</span>
              <span>{{ status.ram }}</span>
            </div>
            <div v-if="status.disk" class="flex items-center gap-1 text-2xs text-ink-2 whitespace-nowrap">
              <HardDrive :size="10" class="text-warn" />
              <span class="font-medium">Disk:</span>
              <span>{{ status.disk }}</span>
            </div>
            <div v-if="status.uptime" class="flex items-center gap-1 text-2xs text-ink-2 whitespace-nowrap">
              <Clock :size="10" class="text-syn-purple" />
              <span class="font-medium">Up:</span>
              <span>{{ status.uptime }}</span>
            </div>
            <div class="flex items-center gap-1 text-2xs text-ink-2 whitespace-nowrap">
              <ArrowDown :size="10" class="text-good" />
              <span>{{ status.netDown || '—' }}</span>
            </div>
            <div class="flex items-center gap-1 text-2xs text-ink-2 whitespace-nowrap">
              <ArrowUp :size="10" class="text-syn-blue" />
              <span>{{ status.netUp || '—' }}</span>
            </div>
          </template>
          <!-- Error -->
          <div v-if="statusError" class="flex items-center gap-1 text-2xs text-bad whitespace-nowrap" :title="statusError">
            <span class="w-1.5 h-1.5 rounded-full bg-bad" />
            <span class="truncate max-w-[200px]">{{ statusError }}</span>
          </div>
        </div>
        <button
          @click.stop="statusExpanded = !statusExpanded"
          class="text-ink-2 hover:text-ink shrink-0 ml-2"
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
      class="fixed z-50 px-2 py-1 bg-surface text-ink text-xs rounded shadow-lg pointer-events-none whitespace-nowrap border border-line"
      :style="{ left: tooltip.x + 'px', top: tooltip.y + 'px' }"
    >
      {{ tooltip.text }}
    </div>

    <!-- Search bar -->
    <div
      v-if="searchVisible"
      class="absolute top-2 right-2 bg-surface border border-line rounded shadow-lg p-2 z-20 flex items-center gap-2"
    >
      <input
        ref="searchInput"
        v-model="searchQuery"
        type="text"
        placeholder="Find..."
        class="bg-input border border-line rounded px-2 py-1 text-xs text-ink w-40 focus:outline-none focus:border-accent placeholder-ink-3"
        @keydown.enter="findNext"
        @keydown.shift.enter="findPrevious"
        @keydown.esc="closeSearch"
      />
      <button
        @click="findPrevious"
        class="text-ink-2 hover:text-ink px-1"
        title="Previous"
      >
        ↑
      </button>
      <button
        @click="findNext"
        class="text-ink-2 hover:text-ink px-1"
        title="Next"
      >
        ↓
      </button>
      <label class="flex items-center gap-1 text-xs text-ink-2 cursor-pointer select-none">
        <input v-model="searchCaseSensitive" type="checkbox" class="accent-accent" />
        Aa
      </label>
      <button @click="closeSearch" class="text-ink-2 hover:text-ink px-1">×</button>
    </div>

    <!-- Disconnect banner -->
    <div
      v-if="isDisconnected"
      class="absolute inset-0 bg-canvas/90 flex flex-col items-center justify-center z-10"
    >
      <p class="text-bad text-base font-semibold mb-3">Connection lost</p>
      <button
        @click="reconnect"
        :disabled="isReconnecting"
        class="px-4 py-1.5 bg-accent-solid hover:bg-accent-solid-hover disabled:bg-input text-ink rounded text-xs font-medium"
      >
        {{ isReconnecting ? 'Reconnecting...' : 'Reconnect' }}
      </button>
    </div>

    <!-- Context menu -->
    <div
      v-if="contextMenu.show"
      ref="contextMenuEl"
      class="fixed bg-surface border border-line rounded shadow-lg py-1 z-50 min-w-[8rem]"
      :style="{ top: contextMenu.y + 'px', left: contextMenu.x + 'px' }"
    >
      <button @click="copySelection" class="block w-full text-left px-3 py-1 text-xs text-ink hover:bg-raised">Copy</button>
      <button @click="pasteFromClipboard" class="block w-full text-left px-3 py-1 text-xs text-ink hover:bg-raised">Paste</button>
      <button @click="selectAll" class="block w-full text-left px-3 py-1 text-xs text-ink hover:bg-raised">Select All</button>
      <div v-if="contextMenuForward" class="border-t border-line my-1"></div>
      <button
        v-if="contextMenuForward"
        @click="createForwardFromSelection"
        class="block w-full text-left px-3 py-1 text-xs text-accent-soft hover:bg-raised"
      >
        {{ contextMenuForward.label }}
      </button>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
import { invoke } from '../utils/invoke.js'
import { writeText, readText } from '@tauri-apps/plugin-clipboard-manager'
import { isInsertableCommand, stripSubmit } from '../utils/terminalInsert.js'
import { Cpu, MemoryStick, HardDrive, Clock, Monitor, ChevronUp, ChevronDown, Loader2, ArrowDown, ArrowUp, FileText, Terminal as TerminalIcon } from 'lucide-vue-next'
import { currentTerminalTheme } from '../composables/useTheme.js'
import { formatBytes } from '../utils/format.js'
import { shellEscape } from '../utils/shell.js'
import { useConnectionStore } from '../stores/connection.js'
import '@xterm/xterm/css/xterm.css'
import { useContextMenu } from '../composables/useContextMenu.js'
import { useListenerGroup } from '../composables/useListenerGroup.js'
import { createTerminalInstance, loadTerminalFontSize } from '../composables/useXtermInstance.js'
import { useHostStatusPolling } from '../composables/useHostStatusPolling.js'

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
let terminal = null // createTerminalInstance() result
let term = null
let fitAddon = null
let searchAddon = null

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
let dockerTerminal = null // createTerminalInstance() result
let dockerTerm = null
let dockerFitAddon = null
const ptyListeners = useListenerGroup()
let dockerKeyFlushTimer = null
const isReconnecting = ref(false)
const { contextMenu, openContextMenu } = useContextMenu(contextMenuEl)
const contextMenuForward = ref(null) // { port, label } or null
const terminalBgClass = ref('bg-canvas')

const searchVisible = ref(false)
const searchQuery = ref('')
const searchCaseSensitive = ref(false)

const statusExpanded = ref(false)
const sysTab = ref('processes')
const {
  status,
  statusLoading,
  statusError,
  startStatusPolling,
  stopStatusPolling,
  resetStatus,
  processes,
  network,
  diskInfo,
  sysLoading,
  startSysPolling,
  stopSysPolling,
} = useHostStatusPolling({ hostId: () => props.hostId, isDisconnected, store })

const visiblePorts = computed(() => network.value?.ports?.slice(0, 8) ?? [])
const visibleInterfaces = computed(() => network.value?.interfaces?.filter(i => i.name !== 'lo').slice(0, 4) ?? [])

const tooltip = ref({ show: false, text: '', x: 0, y: 0 })

function applySettings(settings) {
  if (!term) return
  if (settings.fontSize !== undefined) {
    term.options.fontSize = settings.fontSize
  }
  // Re-read rather than cache: this runs on every settings change,
  // which is exactly when the theme may have flipped.
  term.options.theme = currentTerminalTheme()
  terminalBgClass.value = 'bg-canvas'
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
      await writeText(selection)
    } catch (e) {
      console.warn('Copy failed:', e)
    }
  }
}

async function pasteFromClipboard() {
  contextMenu.value.show = false
  try {
    const text = await readText()
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

function detectForwardInfo(text) {
  if (!text) return null
  const trimmed = text.trim()

  // Match URLs like http://localhost:3000 or https://127.0.0.1:8080/path
  try {
    const url = new URL(trimmed)
    if (url.port) {
      return { port: parseInt(url.port), label: `Forward ${url.hostname}:${url.port}` }
    }
    // Default ports
    if (url.protocol === 'http:') return { port: 80, label: `Forward ${url.hostname}:80` }
    if (url.protocol === 'https:') return { port: 443, label: `Forward ${url.hostname}:443` }
  } catch {
    // not a URL
  }

  // Match host:port like localhost:3000 or 127.0.0.1:8080
  const hostPortMatch = trimmed.match(/^([a-zA-Z0-9._-]+):(\d{2,5})$/)
  if (hostPortMatch) {
    const port = parseInt(hostPortMatch[2])
    return { port, label: `Forward ${hostPortMatch[1]}:${port}` }
  }

  // Match plain port like :3000 or just 3000
  const portMatch = trimmed.match(/^:?(\d{2,5})$/)
  if (portMatch) {
    const port = parseInt(portMatch[1])
    if (port >= 1 && port <= 65535) {
      return { port, label: `Forward port ${port}` }
    }
  }

  return null
}

async function showContextMenu(event) {
  event.preventDefault()
  const selection = term ? term.getSelection() : ''
  contextMenuForward.value = detectForwardInfo(selection)
  await openContextMenu(event)
}

function createForwardFromSelection() {
  contextMenu.value.show = false
  if (!contextMenuForward.value || !props.hostId) return

  const port = contextMenuForward.value.port
  window.dispatchEvent(new CustomEvent('open-port-forward-modal', {
    detail: {
      hostId: props.hostId,
      prefill: {
        name: `Forward ${port}`,
        kind: 'local',
        local_host: '127.0.0.1',
        local_port: port,
        remote_host: 'localhost',
        remote_port: port,
      }
    }
  }))
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

async function openDockerPane({ type, containerName, command }) {
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

  const fontSize = await loadTerminalFontSize()
  dockerTerminal = createTerminalInstance(dockerPaneContainer.value, { fontSize })
  dockerTerm = dockerTerminal.term
  dockerFitAddon = dockerTerminal.fitAddon

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
  await ptyListeners.listen('exec-pty-data', (event) => {
    const payload = event.payload
    if (typeof payload === 'object' && payload.pty_session_id === ptySessionId) {
      dockerTerm.write(payload.data)
    }
  })

  await ptyListeners.listen('exec-pty-error', (event) => {
    const payload = event.payload
    if (typeof payload === 'object' && payload.pty_session_id === ptySessionId) {
      dockerTerm.writeln(`\r\n\x1b[31mError: ${payload.error}\x1b[0m`)
    }
  })

  await ptyListeners.listen('exec-pty-connected', (event) => {
    if (event.payload === ptySessionId) {
      setTimeout(() => {
        if (dockerFitAddon) dockerFitAddon.fit()
      }, 100)
    }
  })

  await ptyListeners.listen('exec-pty-disconnected', (event) => {
    if (event.payload === ptySessionId) {
      dockerPane.value.following = false
      // Auto-close exec panes when the shell exits
      if (dockerPane.value.type === 'exec') {
        closeDockerPane()
      }
    }
  })

  // Observe resize
  dockerTerminal.observeResize(dockerPaneContainer.value)

  // Binary data channel for Docker exec PTY
  dockerTerminal.openDataChannel('open_exec_pty_data_channel', { ptySessionId })

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

  ptyListeners.dispose()

  if (dockerTerminal) {
    dockerTerminal.dispose()
    dockerTerminal = null
  }
  dockerTerm = null
  dockerFitAddon = null

  dockerPane.value.show = false
  dockerPane.value.ptySessionId = null
  dockerPane.value.title = ''
  dockerPane.value.type = null
  dockerPane.value.following = false
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

/**
 * Types a command at this session's prompt. Deliberately sent with no trailing
 * newline: the shell is often root-capable, so the user reviews the line and
 * presses Enter. The guard is re-applied here because this is the last point
 * before the bytes reach the shell.
 */
function onInsertCommand(event) {
  const { sessionId, command } = event.detail || {}
  if (sessionId !== props.sessionId) return
  if (!isInsertableCommand(command)) return
  invoke('ssh_write', { sessionId: props.sessionId, data: stripSubmit(command) }).catch(() => {})
  // Move focus to the terminal so Enter goes to the shell, not the panel.
  term?.focus()
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

async function initTerminal() {
  const fontSize = await loadTerminalFontSize()
  terminalBgClass.value = 'bg-canvas'

  terminal = createTerminalInstance(terminalContainer.value, { fontSize, search: true })
  term = terminal.term
  fitAddon = terminal.fitAddon
  searchAddon = terminal.searchAddon

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
      resetStatus()
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
  terminal.openDataChannel('open_data_channel', { sessionId: props.sessionId })

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
  if (!terminal) return
  // Handle resize
  terminal.observeResize(terminalContainer.value)
}

function stopActiveOperations() {
  stopStatusPolling()
  if (terminal) terminal.unobserveResize()
}

function disposeTerminal() {
  stopActiveOperations()
  stopSysPolling()
  store.unregisterTerminal(props.sessionId)
  if (terminal) {
    terminal.dispose()
    terminal = null
  }
  term = null
  fitAddon = null
  searchAddon = null
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
    // Hand over the tab's SFTP id so the backend can replace that dead
    // handle too, instead of leaving the SFTP panel broken.
    const tab = store.tabs.find(t => t.id === props.sessionId)
    await invoke('ssh_reconnect', {
      sessionId: props.sessionId,
      sftpSessionId: tab?.sftpSessionId ?? null,
    })
  } catch (err) {
    console.error('Reconnect failed:', err)
    isReconnecting.value = false
  }
}

onMounted(async () => {
  await initTerminal()
  startActiveOperations()
  // Fix race condition: if tab is already active when terminal finishes init,
  // the isActive watcher already fired early (term was null). Start polling now.
  if (props.isActive) {
    startStatusPolling()
  }
  document.addEventListener('visibilitychange', onVisibilityChange)
  window.addEventListener('docker-pane-open', onDockerPaneOpen)
  window.addEventListener('terminal-insert-command', onInsertCommand)
})

onUnmounted(() => {
  document.removeEventListener('visibilitychange', onVisibilityChange)
  window.removeEventListener('docker-pane-open', onDockerPaneOpen)
  window.removeEventListener('terminal-insert-command', onInsertCommand)
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
    if (!term) return
    term.focus()
    nextTick(() => {
      requestAnimationFrame(() => {
        if (fitAddon) fitAddon.fit()
        if (dockerFitAddon) dockerFitAddon.fit()
        // Force redraw after tab switch
        if (term) term.refresh(0, term.rows - 1)
        if (dockerTerm) dockerTerm.refresh(0, dockerTerm.rows - 1)
      })
    })
    if (terminal) terminal.observeResize(terminalContainer.value)
    startStatusPolling()
  } else {
    if (term) term.blur()
    stopStatusPolling()
    if (terminal) terminal.unobserveResize()
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