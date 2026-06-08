<template>
  <div v-if="show" class="fixed inset-0 bg-black/50 flex items-center justify-center z-50" @click.self="$emit('close')">
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

      <div class="flex justify-end gap-2 mt-6">
        <button @click="$emit('close')" class="px-4 py-2 text-sm text-[#858585] hover:text-[#cccccc]">Cancel</button>
        <button @click="save" class="px-4 py-2 text-sm bg-[#0e639c] hover:bg-[#1177bb] text-white rounded">Save</button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, watch } from 'vue'
import { useConnectionStore } from '../stores/connection.js'

const props = defineProps({
  show: Boolean,
})

const emit = defineEmits(['close', 'saved'])
const store = useConnectionStore()

const fontSize = ref(14)
const downloadPath = ref('')

watch(() => props.show, async (isOpen) => {
  if (isOpen) {
    await store.loadSettings()
    fontSize.value = parseInt(store.settings.font_size || '14')
    downloadPath.value = store.settings.download_path || ''
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
</script>
