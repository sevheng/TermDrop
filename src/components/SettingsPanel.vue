<template>
  <div v-if="show" class="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
    <div class="bg-[#252526] rounded-lg p-6 w-96 border border-[#3c3c3c] shadow-xl">
      <h3 class="text-lg font-semibold text-[#cccccc] mb-4">Settings</h3>

      <div class="space-y-2">
        <div>
          <label class="block text-xs text-[#858585] mb-1">Terminal Font Size: {{ fontSize }}px</label>
          <input
            v-model.number="fontSize"
            type="range"
            min="10"
            max="24"
            class="w-full accent-[#007acc]"
          />
        </div>

        <div>
          <label class="block text-xs text-[#858585] mb-1">Download Path (leave empty for default)</label>
          <input
            v-model="downloadPath"
            type="text"
            placeholder="~/Downloads"
            class="w-full bg-[#3c3c3c] border border-[#3c3c3c] rounded px-3 py-2 text-sm text-[#cccccc] focus:outline-none focus:border-[#007acc]"
          />
        </div>
      </div>

      <div class="border-t border-[#3c3c3c] pt-4 mt-4">
        <div class="flex items-center justify-between">
          <div>
            <p class="text-xs text-[#cccccc]">Version: <span class="font-mono">{{ appVersion }}</span></p>
            <p v-if="lastChecked" class="text-xs text-[#858585]">Last checked: {{ lastChecked }}</p>
          </div>
          <button
            @click="manualCheck"
            :disabled="checking"
            class="px-3 py-1.5 text-xs bg-[#3c3c3c] hover:bg-[#4c4c4c] text-[#cccccc] rounded disabled:opacity-50"
          >
            {{ checking ? 'Checking...' : 'Check for Updates' }}
          </button>
        </div>
      </div>

      <div class="flex justify-end gap-2 mt-6">
        <button @click="$emit('close')" class="px-4 py-2 text-sm text-[#858585] hover:text-[#cccccc]">Cancel</button>
        <button @click="save" class="px-4 py-2 text-sm bg-[#0e639c] hover:bg-[#1177bb] text-white rounded">Save</button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, watch } from 'vue'
import { getVersion } from '@tauri-apps/api/app'
import { useConnectionStore } from '../stores/connection.js'
import { checkForUpdates } from '../composables/useUpdater.js'

const props = defineProps({
  show: Boolean,
})

const emit = defineEmits(['close', 'saved', 'update-available'])
const store = useConnectionStore()

const fontSize = ref(14)
const downloadPath = ref('')
const appVersion = ref('0.2.3')
const checking = ref(false)
const lastChecked = ref('')

watch(() => props.show, async (isOpen) => {
  if (isOpen) {
    await store.loadSettings()
    fontSize.value = parseInt(store.settings.font_size || '14')
    downloadPath.value = store.settings.download_path || ''
    try {
      appVersion.value = await getVersion()
    } catch {
      appVersion.value = '0.2.3'
    }
  }
})

async function save() {
  await store.saveSettings({
    font_size: String(fontSize.value),
    download_path: downloadPath.value,
  })
  window.dispatchEvent(new CustomEvent('terminal-settings-changed', {
    detail: { fontSize: fontSize.value }
  }))
  emit('saved', { fontSize: fontSize.value, downloadPath: downloadPath.value })
  emit('close')
}

async function manualCheck() {
  checking.value = true
  try {
    const result = await checkForUpdates()
    lastChecked.value = new Date().toLocaleTimeString()
    if (result.available) {
      emit('update-available', result)
    } else {
      window.dispatchEvent(new CustomEvent('app-toast', {
        detail: { message: 'You are on the latest version!', type: 'success' }
      }))
    }
  } catch (err) {
    window.dispatchEvent(new CustomEvent('app-toast', {
      detail: { message: 'Update check failed: ' + err, type: 'error' }
    }))
  } finally {
    checking.value = false
  }
}
</script>
