<template>
  <div v-if="show" class="fixed inset-0 bg-black/50 flex items-center justify-center z-50" @click.self="$emit('close')">
    <div class="bg-gray-800 rounded-lg p-6 w-96 border border-gray-700">
      <h3 class="text-lg font-semibold text-white mb-4">Settings</h3>
      
      <div class="space-y-4">
        <div>
          <label class="block text-xs text-gray-400 mb-1">Terminal Font Size: {{ fontSize }}px</label>
          <input
            v-model.number="fontSize"
            type="range"
            min="10"
            max="24"
            class="w-full accent-blue-500"
          />
        </div>
        
        <div>
          <label class="block text-xs text-gray-400 mb-1">Theme</label>
          <div class="flex gap-2">
            <button
              @click="theme = 'dark'"
              class="flex-1 py-2 text-sm rounded border"
              :class="theme === 'dark' ? 'bg-blue-600 border-blue-500 text-white' : 'bg-gray-700 border-gray-600 text-gray-300'"
            >Dark</button>
            <button
              @click="theme = 'light'"
              class="flex-1 py-2 text-sm rounded border"
              :class="theme === 'light' ? 'bg-blue-600 border-blue-500 text-white' : 'bg-gray-700 border-gray-600 text-gray-300'"
            >Light</button>
          </div>
        </div>
        
        <div>
          <label class="block text-xs text-gray-400 mb-1">Download Path (leave empty for default)</label>
          <input
            v-model="downloadPath"
            type="text"
            placeholder="~/Downloads"
            class="w-full bg-gray-700 border border-gray-600 rounded px-3 py-2 text-sm text-white focus:outline-none focus:border-blue-500"
          />
        </div>
      </div>
      
      <div class="flex justify-end gap-2 mt-6">
        <button @click="$emit('close')" class="px-4 py-2 text-sm text-gray-300 hover:text-white">Cancel</button>
        <button @click="save" class="px-4 py-2 text-sm bg-blue-600 hover:bg-blue-700 text-white rounded">Save</button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'

const props = defineProps({
  show: Boolean,
})

const emit = defineEmits(['close', 'saved'])

const fontSize = ref(14)
const theme = ref('dark')
const downloadPath = ref('')

watch(() => props.show, async (isOpen) => {
  if (isOpen) {
    fontSize.value = parseInt(await invoke('get_setting', { key: 'font_size' }) || '14')
    theme.value = await invoke('get_setting', { key: 'theme' }) || 'dark'
    downloadPath.value = await invoke('get_setting', { key: 'download_path' }) || ''
  }
})

async function save() {
  await invoke('set_setting', { key: 'font_size', value: String(fontSize.value) })
  await invoke('set_setting', { key: 'theme', value: theme.value })
  await invoke('set_setting', { key: 'download_path', value: downloadPath.value })
  emit('saved', { fontSize: fontSize.value, theme: theme.value, downloadPath: downloadPath.value })
  emit('close')
}
</script>
