<template>
  <div class="h-full flex flex-col bg-canvas">
    <!-- Toolbar -->
    <div class="flex items-center justify-between px-2 py-1 border-b border-line">
      <div class="flex items-center gap-2">
        <button
          @click="loadContainers"
          class="text-ink-2 hover:text-ink p-1"
          title="Refresh"
        >
          <RefreshCw :size="12" />
        </button>
        <label class="flex items-center gap-1 text-2xs text-ink-2 cursor-pointer select-none">
          <input
            v-model="showAll"
            type="checkbox"
            class="accent-accent"
            @change="loadContainers"
          />
          Show all
        </label>
      </div>
      <span class="text-2xs text-ink-3">{{ containers.length }} containers</span>
    </div>

    <!-- Container list -->
    <div class="flex-1 min-h-[80px] relative">
      <EmptyState v-if="loading" state="loading" title="Loading containers…" />

      <EmptyState
        v-else-if="dockerNotInstalled"
        state="empty"
        :icon="Container"
        title="Docker is not installed"
        hint="This host does not have Docker available"
        action-label="Install Docker"
        :action-pending="installing"
        pending-label="Installing… this may take a minute"
        @action="installDocker"
      >
        <p class="text-2xs text-ink-3 mt-2 font-mono">curl -fsSL https://get.docker.com | sh</p>
      </EmptyState>

      <EmptyState
        v-else-if="daemonNotRunning"
        state="empty"
        :icon="Container"
        title="Docker daemon is not running"
        hint="Docker is installed but the service is stopped"
      >
        <!-- The command is the answer here, so it stays copyable rather than
             being flattened into prose. -->
        <div class="bg-surface border border-line rounded px-3 py-2 text-left max-w-xs mt-3">
          <p class="text-2xs text-ink-3 mb-1">Start it by running in a terminal:</p>
          <code class="text-2xs text-good font-mono block">sudo systemctl start docker</code>
        </div>
      </EmptyState>

      <EmptyState
        v-else-if="permissionDenied"
        state="empty"
        :icon="Container"
        title="Docker permission denied"
        hint="Your user is not in the docker group"
      >
        <div class="bg-surface border border-line rounded px-3 py-2 text-left max-w-xs mt-3">
          <p class="text-2xs text-ink-3 mb-1">Fix by running in a terminal:</p>
          <code class="text-2xs text-good font-mono block">sudo usermod -aG docker $USER</code>
          <p class="text-2xs text-ink-3 mt-1">Then reconnect this session</p>
        </div>
      </EmptyState>

      <!-- "None running" and "none at all" are different answers, and the
           Show all toggle is what resolves the first. -->
      <EmptyState
        v-else-if="containers.length === 0"
        :state="showAll ? 'empty' : 'filtered'"
        :icon="Container"
        :title="showAll ? 'No containers on this host' : 'No running containers'"
        :hint="showAll ? undefined : 'Stopped containers are hidden'"
        :action-label="showAll ? undefined : 'Show all'"
        @action="showAll = true; loadContainers()"
      />

      <VirtualList
        v-else
        :items="containers"
        :itemHeight="52"
        :keyFn="(c) => c.id"
        class="h-full"
      >
        <template #default="{ item: c }">
          <div
            class="flex items-center gap-2 px-2 py-1 border-b border-line/50 hover:bg-raised"
          >
            <!-- Status dot -->
            <span
              class="w-2 h-2 rounded-full shrink-0"
              :class="c.running ? 'bg-good' : 'bg-ink-3'"
            />
            <!-- Info -->
            <div class="flex-1 min-w-0">
              <div class="text-xs text-ink truncate">{{ c.name }}</div>
              <div class="text-2xs text-ink-2 truncate">{{ c.image }}</div>
              <div class="text-2xs text-ink-3 truncate">{{ c.status }}<span v-if="c.ports"> · {{ c.ports }}</span></div>
            </div>
            <!-- Actions -->
            <div class="flex items-center gap-0.5 shrink-0">
              <button
                v-if="!c.running"
                @click="startContainer(c.id)"
                class="text-ink-2 hover:text-good p-0.5"
                title="Start"
              >
                <Play :size="12" />
              </button>
              <button
                v-if="c.running"
                @click="stopContainer(c.id)"
                class="text-ink-2 hover:text-bad p-0.5"
                title="Stop"
              >
                <Square :size="12" />
              </button>
              <button
                @click="restartContainer(c.id)"
                class="text-ink-2 hover:text-ink p-0.5"
                title="Restart"
              >
                <RotateCcw :size="12" />
              </button>
              <button
                @click="viewLogs(c.id, c.name, c.running)"
                class="text-ink-2 hover:text-ink p-0.5"
                title="Logs"
              >
                <FileText :size="12" />
              </button>
              <button
                @click="execInto(c.id, c.name)"
                class="text-ink-2 hover:text-ink p-0.5"
                title="Exec"
              >
                <Terminal :size="12" />
              </button>
            </div>
          </div>
        </template>
      </VirtualList>
    </div>

    <ConfirmDialog
      :show="confirmDialog.show"
      :title="confirmDialog.title"
      :message="confirmDialog.message"
      :danger="confirmDialog.danger"
      confirm-text="Confirm"
      cancel-text="Cancel"
      @confirm="confirmDialog.onConfirm"
      @cancel="confirmDialog.show = false"
    />
  </div>
</template>

<script setup>
import { ref, onMounted, onUnmounted, onActivated, onDeactivated, watch } from 'vue'
import { invoke } from '../utils/invoke.js'
import {
  RefreshCw, Container,
  Play, Square, RotateCcw, FileText, Terminal,
} from 'lucide-vue-next'
import VirtualList from './VirtualList.vue'
import EmptyState from './EmptyState.vue'
import ConfirmDialog from './ConfirmDialog.vue'
import { shellEscape } from '../utils/shell.js'
import { toast } from '../utils/toast.js'
import { useConfirmDialog } from '../composables/useConfirmDialog.js'

const props = defineProps({
  hostId: {
    type: Number,
    required: true,
  },
})

const emit = defineEmits(['openPane'])

const containers = ref([])
const loading = ref(false)
const showAll = ref(false)
const dockerNotInstalled = ref(false)
const permissionDenied = ref(false)
const daemonNotRunning = ref(false)
const installing = ref(false)
let refreshInterval = null

// Confirm dialog state
const { confirmDialog, openConfirm } = useConfirmDialog()

async function loadContainers(silent = false) {
  if (!props.hostId) return
  if (!silent) loading.value = true
  try {
    containers.value = await invoke('docker_ps', { hostId: props.hostId, all: showAll.value })
    dockerNotInstalled.value = false
    permissionDenied.value = false
    daemonNotRunning.value = false
  } catch (err) {
    const errStr = String(err)
    dockerNotInstalled.value = errStr.includes('DOCKER_NOT_INSTALLED')
    permissionDenied.value = errStr.includes('DOCKER_PERMISSION_DENIED')
    daemonNotRunning.value = errStr.includes('DOCKER_DAEMON_NOT_RUNNING')
    if (!dockerNotInstalled.value && !permissionDenied.value && !daemonNotRunning.value) {
      console.error('docker_ps failed:', err)
    }
    containers.value = []
  }
  if (!silent) loading.value = false
}

function installDocker() {
  if (!props.hostId) return
  // This pipes a script from the internet into a root shell on the remote
  // host, so it needs explicit consent rather than a single click.
  openConfirm({
    title: 'Install Docker',
    message:
      'This runs "curl -fsSL https://get.docker.com | sh" on the remote host, ' +
      'which downloads and executes an installation script with root privileges. Continue?',
    danger: true,
    onConfirm: runDockerInstall,
  })
}

async function runDockerInstall() {
  installing.value = true
  try {
    await invoke('docker_install', { hostId: props.hostId })
    toast('Docker installed successfully', 'success')
    dockerNotInstalled.value = false
    await loadContainers()
  } catch (err) {
    console.error('docker_install failed:', err)
    toast('Docker install failed: ' + err, 'error')
  }
  installing.value = false
}

function handleDockerError(err, action) {
  const errStr = String(err)
  if (errStr.includes('DOCKER_PERMISSION_DENIED')) {
    toast('Docker permission denied. Add user to docker group: sudo usermod -aG docker $USER', 'error')
  } else if (errStr.includes('DOCKER_DAEMON_NOT_RUNNING')) {
    toast('Docker daemon is not running. Start it with: sudo systemctl start docker', 'error')
  } else if (errStr.includes('DOCKER_NOT_INSTALLED')) {
    toast('Docker is not installed on this host', 'error')
  } else {
    toast(`${action} failed: ${errStr}`, 'error')
  }
}

async function startContainer(id) {
  const c = containers.value.find(x => x.id === id)
  if (c) { c.running = true; c.status = 'Starting...' }
  try {
    await invoke('docker_start', { hostId: props.hostId, containerId: id })
    loadContainers(true)
  } catch (err) {
    handleDockerError(err, 'Start')
    loadContainers(true)
  }
}

async function stopContainer(id) {
  const c = containers.value.find(x => x.id === id)
  if (!c) return
  openConfirm({
    title: 'Stop Container',
    message: `Stop "${c.name}"?`,
    danger: true,
    onConfirm: async () => {
      c.running = false
      c.status = 'Stopping...'
      try {
        await invoke('docker_stop', { hostId: props.hostId, containerId: id })
        loadContainers(true)
      } catch (err) {
        handleDockerError(err, 'Stop')
        loadContainers(true)
      }
    },
  })
}

async function restartContainer(id) {
  const c = containers.value.find(x => x.id === id)
  if (!c) return
  openConfirm({
    title: 'Restart Container',
    message: `Restart "${c.name}"?`,
    danger: true,
    onConfirm: async () => {
      c.status = 'Restarting...'
      try {
        await invoke('docker_restart', { hostId: props.hostId, containerId: id })
        loadContainers(true)
      } catch (err) {
        handleDockerError(err, 'Restart')
        loadContainers(true)
      }
    },
  })
}

async function viewLogs(id, name, running) {
  const cmd = running
    ? `docker logs -f --tail 200 ${shellEscape(name)}`
    : `docker logs --tail 200 ${shellEscape(name)}`
  emit('openPane', { type: 'logs', containerId: id, containerName: name, command: cmd })
}

async function execInto(id, name) {
  try {
    const shell = await invoke('docker_inspect_shell', { hostId: props.hostId, containerId: id })
    const cmd = `docker exec -it ${shellEscape(name)} ${shell}`
    emit('openPane', { type: 'exec', containerId: id, containerName: name, command: cmd })
  } catch (err) {
    handleDockerError(err, 'Exec')
  }
}

function startAutoRefresh() {
  stopAutoRefresh()
  refreshInterval = setInterval(() => {
    if (!document.hidden && props.hostId) {
      loadContainers(true)
    }
  }, 5000)
}

function stopAutoRefresh() {
  if (refreshInterval) {
    clearInterval(refreshInterval)
    refreshInterval = null
  }
}

onMounted(() => {
  loadContainers()
  startAutoRefresh()
})

onUnmounted(() => {
  stopAutoRefresh()
})

// Under <KeepAlive> a hidden panel is deactivated, not unmounted, so without
// these the 5s docker ps poll kept running for every panel ever opened, each
// one holding that host's exec session while the user looked elsewhere.
onDeactivated(stopAutoRefresh)
onActivated(() => {
  loadContainers(true)
  startAutoRefresh()
})

watch(() => props.hostId, () => {
  loadContainers()
  startAutoRefresh()
})
</script>
