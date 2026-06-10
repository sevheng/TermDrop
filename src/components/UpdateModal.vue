<template>
  <div v-if="show" class="fixed inset-0 bg-black/60 flex items-center justify-center z-[100]">
    <div class="bg-[#252526] rounded-lg p-6 w-[420px] border border-[#3c3c3c] shadow-2xl">
      <div class="flex items-center gap-2 mb-3">
        <span class="text-2xl">🎉</span>
        <h3 class="text-lg font-semibold text-[#cccccc]">Update Available</h3>
      </div>

      <p class="text-sm text-[#cccccc] mb-2">
        <strong>TermDrop v{{ version }}</strong> is now available.
      </p>

      <div v-if="notes" class="bg-[#1e1e1e] rounded p-3 mb-4 max-h-40 overflow-y-auto text-xs text-[#aaaaaa] leading-relaxed">
        <pre class="whitespace-pre-wrap font-mono">{{ notes }}</pre>
      </div>

      <div v-if="downloading" class="mb-4">
        <div class="flex justify-between text-xs text-[#858585] mb-1">
          <span>Downloading...</span>
          <span>{{ progress }}%</span>
        </div>
        <div class="w-full bg-[#3c3c3c] rounded-full h-2">
          <div class="bg-[#0e639c] h-2 rounded-full transition-all duration-200" :style="{ width: progress + '%' }"></div>
        </div>
      </div>

      <div class="flex justify-end gap-2">
        <button
          @click="$emit('close')"
          class="px-4 py-2 text-sm text-[#858585] hover:text-[#cccccc] rounded"
          :disabled="downloading"
        >Later</button>
        <button
          @click="install"
          class="px-4 py-2 text-sm bg-[#0e639c] hover:bg-[#1177bb] text-white rounded disabled:opacity-50"
          :disabled="downloading"
        >{{ downloading ? 'Installing...' : 'Download & Install' }}</button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref } from 'vue'

const props = defineProps({
  show: Boolean,
  version: String,
  notes: String,
  downloadAndInstall: Function,
})

const emit = defineEmits(['close', 'installed'])

const downloading = ref(false)
const progress = ref(0)

async function install() {
  if (!props.downloadAndInstall) return
  downloading.value = true
  progress.value = 0
  try {
    await props.downloadAndInstall((p) => { progress.value = p })
    emit('installed')
  } catch (err) {
    window.dispatchEvent(new CustomEvent('app-toast', {
      detail: { message: 'Update failed: ' + err, type: 'error' }
    }))
    downloading.value = false
  }
}
</script>
