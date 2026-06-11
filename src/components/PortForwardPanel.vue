<template>
  <div class="w-72 h-full bg-[#252526] border-l border-[#3c3c3c] flex flex-col">
    <!-- Header -->
    <div class="px-3 py-2 border-b border-[#3c3c3c] flex items-center justify-between">
      <h3 class="text-xs font-semibold text-[#cccccc]">Port Forwards</h3>
      <button
        @click="$emit('add')"
        class="text-[#858585] hover:text-[#cccccc] p-1"
        title="Add forward"
      >
        <Plus :size="12" />
      </button>
    </div>

    <!-- List -->
    <div class="flex-1 overflow-y-auto py-1 px-2">
      <div v-if="forwards.length === 0" class="flex flex-col items-center justify-center py-8 text-[#6e6e6e]">
        <Network :size="20" class="mb-2 opacity-50" />
        <p class="text-xs">No port forwards</p>
        <p class="text-xs mt-1">Click + to add one</p>
      </div>

      <div v-for="fw in forwards" :key="fw.id" class="mb-2">
        <div class="bg-[#252526] rounded p-2 border border-[#3c3c3c]">
          <div class="flex items-center justify-between mb-1">
            <span class="text-xs font-medium text-[#cccccc] truncate">{{ fw.name }}</span>
            <span
              class="text-[10px] px-1.5 py-0.5 rounded font-medium"
              :class="activeStatus[fw.id] ? 'bg-[#89d185]/20 text-[#89d185]' : 'bg-[#3c3c3c] text-[#858585]'"
            >
              {{ activeStatus[fw.id] ? 'Active' : 'Stopped' }}
            </span>
          </div>

          <div class="text-[10px] text-[#858585] space-y-0.5">
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
              class="flex-1 text-[10px] bg-[#0e639c] hover:bg-[#1177bb] text-white py-1 rounded transition-colors"
            >
              Start
            </button>
            <button
              v-else
              @click="stopForward(fw.id)"
              class="flex-1 text-[10px] bg-[#3c3c3c] hover:bg-[#37373d] text-[#cccccc] py-1 rounded transition-colors"
            >
              Stop
            </button>
            <button
              v-if="activeStatus[fw.id] && fw.kind === 'local'"
              @click="openForward(fw)"
              class="text-[10px] bg-[#0e639c]/10 hover:bg-[#0e639c]/20 text-[#75beff] py-1 px-2 rounded transition-colors"
              title="Open in browser"
            >
              <ExternalLink :size="10" />
            </button>
            <button
              @click="deleteForward(fw.id)"
              class="text-[10px] bg-[#f44336]/10 hover:bg-[#f44336]/20 text-[#f44336] py-1 px-2 rounded transition-colors"
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
import { ref, watch, onMounted } from 'vue'
import { useConnectionStore } from '../stores/connection.js'
import { Plus, Network, ArrowRightLeft, ArrowRight, Trash2, ExternalLink } from 'lucide-vue-next'
import { openUrl } from '@tauri-apps/plugin-opener'

const props = defineProps({
  hostId: Number,
})

const emit = defineEmits(['add'])

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
    window.dispatchEvent(new CustomEvent('app-toast', { detail: { message: 'Port forward started', type: 'success' } }))
  } catch (err) {
    window.dispatchEvent(new CustomEvent('app-toast', { detail: { message: 'Failed to start: ' + err, type: 'error' } }))
  }
}

function openForward(fw) {
  const url = `http://${fw.local_host}:${fw.local_port}`
  openUrl(url).catch((err) => {
    window.dispatchEvent(new CustomEvent('app-toast', { detail: { message: 'Failed to open: ' + err, type: 'error' } }))
  })
}

async function stopForward(id) {
  try {
    await store.stopPortForward(id)
    activeStatus.value[id] = false
    window.dispatchEvent(new CustomEvent('app-toast', { detail: { message: 'Port forward stopped', type: 'success' } }))
  } catch (err) {
    window.dispatchEvent(new CustomEvent('app-toast', { detail: { message: 'Failed to stop: ' + err, type: 'error' } }))
  }
}

async function deleteForward(id) {
  try {
    await store.deletePortForward(id)
    await loadForwards()
    window.dispatchEvent(new CustomEvent('app-toast', { detail: { message: 'Port forward deleted', type: 'success' } }))
  } catch (err) {
    window.dispatchEvent(new CustomEvent('app-toast', { detail: { message: 'Failed to delete: ' + err, type: 'error' } }))
  }
}

watch(() => props.hostId, loadForwards, { immediate: true })

onMounted(loadForwards)
</script>
