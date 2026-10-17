<template>
  <div class="fixed top-4 right-4 z-100 space-y-2 pointer-events-none">
    <div
      v-for="toast in toasts"
      :key="toast.id"
      class="px-4 py-3 rounded-sm shadow-lg text-sm max-w-sm pointer-events-auto flex items-start gap-3"
      :class="toastClass(toast.type)"
    >
      <span class="flex-1">{{ toast.message }}</span>
      <span v-if="toast.count > 1" class="shrink-0 text-white/70 tabular-nums">×{{ toast.count }}</span>
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
import { useListenerGroup } from '../composables/useListenerGroup.js'

const toasts = ref([])
const listeners = useListenerGroup()
let timerMap = new Map()

function toastClass(type) {
  switch (type) {
    case 'error':
      return 'bg-bad-solid text-white'
    case 'success':
      return 'bg-good-solid text-white'
    case 'warning':
      return 'bg-warn-solid text-white'
    case 'info':
    default:
      return 'bg-accent-solid text-white'
  }
}

function addToast(message, type = 'error', duration = 4000) {
  // A repeat of a toast still on screen counts up instead of stacking. A remote
  // desktop rings its bell once per BEL, so `yes $'\a'` used to fill the
  // window with identical toasts and bury the toolbar under them.
  const existing = toasts.value.find(t => t.message === message && t.type === type)
  if (existing) {
    existing.count += 1
    clearTimeout(timerMap.get(existing.id))
    timerMap.set(existing.id, setTimeout(() => dismiss(existing.id), duration))
    return
  }
  const id = Date.now() + Math.random()
  toasts.value.push({ id, message, type, count: 1 })
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
  await listeners.listen('ssh-error', (event) => {
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
  window.removeEventListener('app-toast', onAppToast)
  timerMap.forEach(t => clearTimeout(t))
  timerMap.clear()
})
</script>
