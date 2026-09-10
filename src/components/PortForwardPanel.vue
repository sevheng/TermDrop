<template>
  <div class="w-72 h-full bg-surface border-l border-line flex flex-col">
    <!-- Header -->
    <div class="px-3 py-2 border-b border-line flex items-center justify-between">
      <h3 class="text-xs font-semibold text-ink">Port Forwards</h3>
      <button
        @click="$emit('add')"
        class="text-ink-2 hover:text-ink p-1"
        title="Add forward"
      >
        <Plus :size="12" />
      </button>
    </div>

    <!-- List -->
    <div class="flex-1 overflow-y-auto py-1 px-2">
      <div v-if="forwards.length === 0" class="flex flex-col items-center justify-center py-8 text-ink-3">
        <Network :size="20" class="mb-2 opacity-50" />
        <p class="text-xs">No port forwards</p>
        <p class="text-xs mt-1">Click + to add one</p>
      </div>

      <div v-for="fw in forwards" :key="fw.id" class="mb-2">
        <div class="bg-surface rounded p-2 border border-line">
          <div class="flex items-center justify-between mb-1">
            <span class="text-xs font-medium text-ink truncate">{{ fw.name }}</span>
            <span
              class="text-[10px] px-1.5 py-0.5 rounded font-medium"
              :class="activeStatus[fw.id] ? 'bg-good/20 text-good' : 'bg-input text-ink-2'"
            >
              {{ activeStatus[fw.id] ? 'Active' : 'Stopped' }}
            </span>
          </div>

          <div class="text-[10px] text-ink-2 space-y-0.5">
            <div class="flex items-center gap-1">
              <ArrowRightLeft :size="9" />
              <span>{{ fw.kind === 'local' ? 'Local' : 'SOCKS' }} → {{ fw.local_host }}:{{ fw.local_port }}</span>
            </div>
            <div v-if="fw.kind === 'local'" class="flex items-center gap-1">
              <ArrowRight :size="9" />
              <span>{{ fw.remote_host }}:{{ fw.remote_port }}</span>
            </div>
          </div>

          <div class="flex gap-1 mt-2">
            <button
              v-if="!activeStatus[fw.id]"
              @click="startForward(fw.id)"
              class="flex-1 text-[10px] bg-accent-solid hover:bg-accent-solid-hover text-white py-1 rounded transition-colors"
            >
              Start
            </button>
            <button
              v-else
              @click="stopForward(fw.id)"
              class="flex-1 text-[10px] bg-input hover:bg-active text-ink py-1 rounded transition-colors"
            >
              Stop
            </button>
            <button
              @click="editForward(fw)"
              class="text-[10px] bg-input hover:bg-input-hover text-ink-2 hover:text-ink py-1 px-2 rounded transition-colors"
              title="Edit"
            >
              <Pencil :size="10" />
            </button>
            <button
              v-if="activeStatus[fw.id] && fw.kind === 'local'"
              @click="openForward(fw)"
              class="text-[10px] bg-accent-solid/10 hover:bg-accent-solid/20 text-accent-soft py-1 px-2 rounded transition-colors"
              title="Open in browser"
            >
              <ExternalLink :size="10" />
            </button>
            <button
              @click="deleteForward(fw.id)"
              class="text-[10px] bg-bad/10 hover:bg-bad/20 text-bad py-1 px-2 rounded transition-colors"
            >
              <Trash2 :size="10" />
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, watch, onMounted, onUnmounted } from 'vue'
import { useConnectionStore } from '../stores/connection.js'
import { Plus, Network, ArrowRightLeft, ArrowRight, Trash2, ExternalLink, Pencil } from 'lucide-vue-next'
import { openUrl } from '@tauri-apps/plugin-opener'
import { toast } from '../utils/toast.js'

const props = defineProps({
  hostId: Number,
})

defineEmits(['add'])

const store = useConnectionStore()
const forwards = ref([])
const activeStatus = ref({})

async function loadForwards() {
  if (!props.hostId) return
  forwards.value = await store.getPortForwards(props.hostId)
  // Check status for each
  for (const fw of forwards.value) {
    activeStatus.value[fw.id] = await store.getPortForwardStatus(fw.id)
  }
}

async function startForward(id) {
  try {
    await store.startPortForward(id)
    activeStatus.value[id] = true
    toast('Port forward started', 'success')
  } catch (err) {
    toast('Failed to start: ' + err, 'error')
  }
}

function openForward(fw) {
  const url = `http://${fw.local_host}:${fw.local_port}`
  openUrl(url).catch((err) => {
    toast('Failed to open: ' + err, 'error')
  })
}

function editForward(fw) {
  window.dispatchEvent(new CustomEvent('edit-port-forward', { detail: { hostId: props.hostId, forward: fw } }))
}

async function stopForward(id) {
  try {
    await store.stopPortForward(id)
    activeStatus.value[id] = false
    toast('Port forward stopped', 'success')
  } catch (err) {
    toast('Failed to stop: ' + err, 'error')
  }
}

async function deleteForward(id) {
  try {
    await store.deletePortForward(id)
    await loadForwards()
    toast('Port forward deleted', 'success')
  } catch (err) {
    toast('Failed to delete: ' + err, 'error')
  }
}

function onPortForwardAdded(event) {
  if (event.detail.hostId === props.hostId) {
    loadForwards()
  }
}

watch(() => props.hostId, loadForwards, { immediate: true })

onMounted(() => {
  loadForwards()
  window.addEventListener('port-forward-added', onPortForwardAdded)
})

onUnmounted(() => {
  window.removeEventListener('port-forward-added', onPortForwardAdded)
})
</script>
