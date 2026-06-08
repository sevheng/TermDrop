<template>
  <div class="h-full flex flex-col bg-[#1e1e1e]">
    <!-- Toolbar -->
    <div class="flex items-center justify-between px-2 py-1 border-b border-[#3c3c3c]">
      <div class="flex items-center gap-2">
        <button
          @click="loadContainers"
          class="text-[#858585] hover:text-[#cccccc] p-1"
          title="Refresh"
        >
          <RefreshCw :size="12" />
        </button>
        <label class="flex items-center gap-1 text-[10px] text-[#858585] cursor-pointer select-none">
          <input
            v-model="showAll"
            type="checkbox"
            class="accent-[#007acc]"
            @change="loadContainers"
          />
          Show all
        </label>
      </div>
      <span class="text-[10px] text-[#6e6e6e]">{{ containers.length }} containers</span>
    </div>

    <!-- Container list -->
    <div class="flex-1 overflow-y-auto">
      <div v-if="loading" class="flex items-center justify-center py-8">
        <Loader2 :size="16" class="animate-spin text-[#858585]" />
      </div>
      <div v-else-if="dockerNotInstalled" class="flex flex-col items-center justify-center py-8 px-4 text-center">
        <Container :size="28" class="mb-3 text-[#6e6e6e] opacity-50" />
        <p class="text-xs text-[#cccccc] mb-1">Docker is not installed</p>
        <p class="text-[10px] text-[#858585] mb-3">This host does not have Docker available</p>
        <button
          v-if="!installing"
          @click="installDocker"
          class="px-3 py-1.5 bg-[#0e639c] hover:bg-[#1177bb] text-white text-xs rounded font-medium"
        >
          Install Docker
        </button>
        <div v-else class="flex items-center gap-2 text-[10px] text-[#858585]">
          <Loader2 :size="14" class="animate-spin" />
          <span>Installing Docker... this may take a minute</span>
        </div>
        <p class="text-[10px] text-[#6e6e6e] mt-2">Runs: curl -fsSL https://get.docker.com | sh</p>
      </div>
      <div v-else-if="containers.length === 0" class="flex flex-col items-center justify-center py-8 text-[#6e6e6e]">
        <Container :size="24" class="mb-2 opacity-50" />
        <p class="text-xs">No containers</p>
        <p class="text-[10px] mt-1">Connect to a host with Docker</p>
      </div>
      <div v-else>
        <div
          v-for="c in containers"
          :key="c.id"
          class="flex items-center gap-2 px-2 py-1 border-b border-[#3c3c3c]/50 hover:bg-[#2a2d2e]"
        >
          <!-- Status dot -->
          <span
            class="w-2 h-2 rounded-full shrink-0"
            :class="c.running ? 'bg-[#89d185]' : 'bg-[#6e6e6e]'"
          />
          <!-- Info -->
          <div class="flex-1 min-w-0">
            <div class="text-[11px] text-[#cccccc] truncate">{{ c.name }}</div>
            <div class="text-[10px] text-[#858585] truncate">{{ c.image }}</div>
            <div class="text-[10px] text-[#6e6e6e] truncate">{{ c.status }}<span v-if="c.ports"> · {{ c.ports }}</span></div>
          </div>
          <!-- Actions -->
          <div class="flex items-center gap-0.5 shrink-0">
            <button
              v-if="!c.running"
              @click="startContainer(c.id)"
              class="text-[#858585] hover:text-[#89d185] p-0.5"
              title="Start"
            >
              <Play :size="12" />
            </button>
            <button
              v-if="c.running"
              @click="stopContainer(c.id)"
              class="text-[#858585] hover:text-[#f44336] p-0.5"
              title="Stop"
            >
              <Square :size="12" />
            </button>
            <button
              @click="restartContainer(c.id)"
              class="text-[#858585] hover:text-[#cccccc] p-0.5"
              title="Restart"
            >
              <RotateCcw :size="12" />
            </button>
            <button
              @click="viewLogs(c.id, c.name)"
              class="text-[#858585] hover:text-[#cccccc] p-0.5"
              title="Logs"
            >
              <FileText :size="12" />
            </button>
            <button
              @click="execInto(c.id, c.name)"
              class="text-[#858585] hover:text-[#cccccc] p-0.5"
              title="Exec"
            >
              <Terminal :size="12" />
            </button>
          </div>
        </div>
      </div>
    </div>

    <!-- Logs modal -->
    <div
      v-if="logModal.show"
      class="fixed inset-0 bg-black/50 flex items-center justify-center z-50"
      @click.self="logModal.show = false"
    >
      <div class="bg-[#252526] border border-[#3c3c3c] rounded shadow-xl w-[32rem] h-[24rem] flex flex-col">
        <div class="flex items-center justify-between px-3 py-2 border-b border-[#3c3c3c]">
          <span class="text-xs text-[#cccccc]">Logs: {{ logModal.containerName }}</span>
          <button @click="logModal.show = false" class="text-[#858585] hover:text-[#cccccc]">×</button>
        </div>
        <div class="flex-1 overflow-auto p-2">
          <pre class="text-[10px] text-[#cccccc] font-mono whitespace-pre-wrap">{{ logModal.content }}</pre>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  RefreshCw, Loader2, Container,
  Play, Square, RotateCcw, FileText, Terminal,
} from 'lucide-vue-next'

const props = defineProps({
  hostId: {
    type: Number,
    required: true,
  },
})

const emit = defineEmits(['exec'])

const containers = ref([])
const loading = ref(false)
const showAll = ref(false)
const dockerNotInstalled = ref(false)
const installing = ref(false)
const logModal = ref({ show: false, containerName: '', content: '' })

async function loadContainers() {
  if (!props.hostId) return
  loading.value = true
  dockerNotInstalled.value = false
  try {
    containers.value = await invoke('docker_ps', { hostId: props.hostId, all: showAll.value })
  } catch (err) {
    const errStr = String(err)
    if (errStr.includes('DOCKER_NOT_INSTALLED')) {
      dockerNotInstalled.value = true
    } else {
      console.error('docker_ps failed:', err)
    }
    containers.value = []
  }
  loading.value = false
}

async function installDocker() {
  if (!props.hostId) return
  installing.value = true
  try {
    await invoke('docker_install', { hostId: props.hostId })
    window.dispatchEvent(new CustomEvent('app-toast', { detail: { message: 'Docker installed successfully', type: 'success' } }))
    dockerNotInstalled.value = false
    await loadContainers()
  } catch (err) {
    console.error('docker_install failed:', err)
    window.dispatchEvent(new CustomEvent('app-toast', { detail: { message: 'Docker install failed: ' + err, type: 'error' } }))
  }
  installing.value = false
}

async function startContainer(id) {
  try {
    await invoke('docker_start', { hostId: props.hostId, containerId: id })
    await loadContainers()
  } catch (err) {
    console.error('docker_start failed:', err)
  }
}

async function stopContainer(id) {
  try {
    await invoke('docker_stop', { hostId: props.hostId, containerId: id })
    await loadContainers()
  } catch (err) {
    console.error('docker_stop failed:', err)
  }
}

async function restartContainer(id) {
  try {
    await invoke('docker_restart', { hostId: props.hostId, containerId: id })
    await loadContainers()
  } catch (err) {
    console.error('docker_restart failed:', err)
  }
}

async function viewLogs(id, name) {
  try {
    const content = await invoke('docker_logs', { hostId: props.hostId, containerId: id, tail: 200 })
    logModal.value = { show: true, containerName: name, content }
  } catch (err) {
    console.error('docker_logs failed:', err)
  }
}

async function execInto(id, name) {
  try {
    const shell = await invoke('docker_inspect_shell', { hostId: props.hostId, containerId: id })
    emit('exec', { containerId: id, containerName: name, shell })
  } catch (err) {
    console.error('docker_inspect_shell failed:', err)
  }
}

onMounted(loadContainers)

watch(() => props.hostId, loadContainers)
</script>
