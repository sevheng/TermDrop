<template>
  <ModalShell :show="show" dim="bg-black/50" z="z-50" panel-class="p-6 w-96 shadow-xl">
      <h3 class="text-lg font-semibold text-ink mb-4">Settings</h3>

      <div class="space-y-2">
        <div>
          <label class="block text-xs text-ink-2 mb-1">Terminal Font Size: {{ fontSize }}px</label>
          <input
            v-model.number="fontSize"
            type="range"
            min="10"
            max="24"
            class="w-full accent-accent"
          />
        </div>

        <div>
          <label class="block text-xs text-ink-2 mb-1">Download Path (leave empty for default)</label>
          <input
            v-model="downloadPath"
            type="text"
            placeholder="~/Downloads"
            class="w-full bg-input border border-line rounded px-3 py-2 text-sm text-ink focus:outline-none focus:border-accent"
          />
        </div>
      </div>

      <div class="border-t border-line pt-4 mt-4">
        <div class="flex items-center justify-between">
          <div>
            <p class="text-xs text-ink">Version: <span class="font-mono">{{ appVersion }}</span></p>
            <p v-if="lastChecked" class="text-xs text-ink-2">Last checked: {{ lastChecked }}</p>
          </div>
          <button
            @click="manualCheck"
            :disabled="checking"
            class="px-3 py-1.5 text-xs bg-input hover:bg-input-hover text-ink rounded disabled:opacity-50"
          >
            {{ checking ? 'Checking...' : 'Check for Updates' }}
          </button>
        </div>
      </div>

      <div class="flex justify-end gap-2 mt-6">
        <button @click="$emit('close')" class="px-4 py-2 text-sm text-ink-2 hover:text-ink">Cancel</button>
        <button @click="save" class="px-4 py-2 text-sm bg-accent-solid hover:bg-accent-solid-hover text-white rounded">Save</button>
      </div>
  </ModalShell>
</template>

<script setup>
import { ref, watch } from 'vue'
import { getVersion } from '@tauri-apps/api/app'
import { useConnectionStore } from '../stores/connection.js'
import { checkForUpdates } from '../composables/useUpdater.js'
import { toast } from '../utils/toast.js'
import ModalShell from './ModalShell.vue'

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
      toast('You are on the latest version!', 'success')
    }
  } catch (err) {
    toast('Update check failed: ' + err, 'error')
  } finally {
    checking.value = false
  }
}
</script>
