<template>
  <div class="fixed top-4 right-4 z-[100] space-y-2 pointer-events-none">
    <div
      v-for="toast in toasts"
      :key="toast.id"
      class="px-4 py-3 rounded shadow-lg text-sm max-w-sm pointer-events-auto flex items-start gap-3"
      :class="toastClass(toast.type)"
    >
      <span class="flex-1">{{ toast.message }}</span>
      <button
        @click="dismiss(toast.id)"
        class="shrink-0 text-white/70 hover:text-white leading-none"
      >
        ×
      </button>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, onUnmounted } from 'vue'
import { listen } from '@tauri-apps/api/event'

const toasts = ref([])
let unlistenError = null
let timerMap = new Map()

function toastClass(type) {
  switch (type) {
    case 'error':
      return 'bg-red-600 text-white'
    case 'success':
      return 'bg-green-600 text-white'
    case 'warning':
      return 'bg-yellow-600 text-white'
    case 'info':
    default:
      return 'bg-blue-600 text-white'
  }
}

function addToast(message, type = 'error', duration = 4000) {
  const id = Date.now() + Math.random()
  toasts.value.push({ id, message, type })
  const timer = setTimeout(() => dismiss(id), duration)
  timerMap.set(id, timer)
}

function dismiss(id) {
  const timer = timerMap.get(id)
  if (timer) {
    clearTimeout(timer)
    timerMap.delete(id)
  }
  toasts.value = toasts.value.filter(t => t.id !== id)
}

function onAppToast(event) {
  const detail = event.detail
  if (detail && detail.message) {
    addToast(detail.message, detail.type || 'info', detail.duration || 4000)
  }
}

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

  window.addEventListener('app-toast', onAppToast)
})

onUnmounted(() => {
  if (unlistenError) unlistenError()
  window.removeEventListener('app-toast', onAppToast)
  timerMap.forEach(t => clearTimeout(t))
  timerMap.clear()
})
</script>
