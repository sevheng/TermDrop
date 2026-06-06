<template>
  <div class="fixed top-4 right-4 z-50 space-y-2">
    <div
      v-for="toast in toasts"
      :key="toast.id"
      class="px-4 py-3 rounded shadow-lg text-sm max-w-sm transition-opacity"
      :class="toast.type === 'error' ? 'bg-red-600 text-white' : 'bg-blue-600 text-white'"
    >
      {{ toast.message }}
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, onUnmounted } from 'vue'
import { listen } from '@tauri-apps/api/event'

const toasts = ref([])
let unlistenError = null

onMounted(async () => {
  unlistenError = await listen('ssh-error', (event) => {
    const payload = event.payload
    let message = 'SSH error'
    if (typeof payload === 'object' && payload.error) {
      message = payload.error
    } else if (typeof payload === 'string') {
      message = payload
    }
    addToast(message, 'error')
  })
})

onUnmounted(() => {
  if (unlistenError) unlistenError()
})

function addToast(message, type = 'error') {
  const id = Date.now() + Math.random()
  toasts.value.push({ id, message, type })
  setTimeout(() => {
    toasts.value = toasts.value.filter(t => t.id !== id)
  }, 4000)
}
</script>
