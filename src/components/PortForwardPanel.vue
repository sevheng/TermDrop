<template>
  <div class="w-72 h-full bg-white border-l border-gray-200 flex flex-col dark:bg-gray-800 dark:border-gray-700">
    <!-- Header -->
    <div class="px-3 py-2 border-b border-gray-200 flex items-center justify-between dark:border-gray-700">
      <h3 class="text-xs font-semibold text-gray-800 dark:text-gray-200">Port Forwards</h3>
      <button
        @click="$emit('add')"
        class="text-gray-500 hover:text-gray-900 p-1 dark:text-gray-400 dark:hover:text-white"
        title="Add forward"
      >
        <Plus :size="12" />
      </button>
    </div>

    <!-- List -->
    <div class="flex-1 overflow-y-auto py-1 px-2">
      <div v-if="forwards.length === 0" class="flex flex-col items-center justify-center py-8 text-gray-400 dark:text-gray-500">
        <Network :size="20" class="mb-2 opacity-50" />
        <p class="text-xs">No port forwards</p>
        <p class="text-xs mt-1">Click + to add one</p>
      </div>

      <div v-for="fw in forwards" :key="fw.id" class="mb-2">
        <div class="bg-gray-50 rounded p-2 border border-gray-200 dark:bg-gray-700/50 dark:border-gray-600">
          <div class="flex items-center justify-between mb-1">
            <span class="text-xs font-medium text-gray-800 dark:text-gray-200 truncate">{{ fw.name }}</span>
            <span
              class="text-[10px] px-1.5 py-0.5 rounded font-medium"
              :class="activeStatus[fw.id] ? 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400' : 'bg-gray-100 text-gray-500 dark:bg-gray-700 dark:text-gray-400'"
            >
              {{ activeStatus[fw.id] ? 'Active' : 'Stopped' }}
            </span>
          </div>

          <div class="text-[10px] text-gray-500 dark:text-gray-400 space-y-0.5">
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
              class="flex-1 text-[10px] bg-blue-600 hover:bg-blue-700 text-white py-1 rounded transition-colors"
            >
              Start
            </button>
            <button
              v-else
              @click="stopForward(fw.id)"
              class="flex-1 text-[10px] bg-gray-200 hover:bg-gray-300 text-gray-700 py-1 rounded transition-colors dark:bg-gray-600 dark:hover:bg-gray-500 dark:text-gray-200"
            >
              Stop
            </button>
            <button
              @click="deleteForward(fw.id)"
              class="text-[10px] bg-red-50 hover:bg-red-100 text-red-600 py-1 px-2 rounded transition-colors dark:bg-red-900/20 dark:hover:bg-red-900/30 dark:text-red-400"
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
import { Plus, Network, ArrowRightLeft, ArrowRight, Trash2 } from 'lucide-vue-next'

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
