<template>
  <div class="relative w-full h-full">
    <div ref="terminalContainer" class="w-full h-full bg-gray-900"></div>

    <!-- Disconnect banner -->
    <div
      v-if="isDisconnected"
      class="absolute inset-0 bg-gray-900/90 flex flex-col items-center justify-center z-10"
    >
      <p class="text-red-400 text-lg font-semibold mb-4">Connection lost</p>
      <button
        @click="reconnect"
        :disabled="isReconnecting"
        class="px-6 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-gray-600 text-white rounded text-sm font-medium"
      >
        {{ isReconnecting ? 'Reconnecting...' : 'Reconnect' }}
      </button>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, onUnmounted, watch } from 'vue'
import { Terminal } from 'xterm'
import { FitAddon } from 'xterm-addon-fit'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import 'xterm/css/xterm.css'

const props = defineProps({
  sessionId: {
    type: String,
    required: true,
  },
})

const terminalContainer = ref(null)
let term = null
let fitAddon = null
let unlistenData = null
let unlistenError = null
let unlistenDisconnected = null
let unlistenReconnected = null
let resizeObserver = null

const isDisconnected = ref(false)
const isReconnecting = ref(false)

onMounted(async () => {
  const fontSizeSetting = await invoke('get_setting', { key: 'font_size' })
  const fontSize = fontSizeSetting ? parseInt(fontSizeSetting) : 14

  term = new Terminal({
    cursorBlink: true,
    fontSize,
    fontFamily: 'Menlo, Monaco, "Courier New", monospace',
    theme: {
      background: '#111827',
      foreground: '#e5e7eb',
      cursor: '#e5e7eb',
      selectionBackground: '#374151',
      black: '#1f2937',
      red: '#ef4444',
      green: '#22c55e',
      yellow: '#eab308',
      blue: '#3b82f6',
      magenta: '#a855f7',
      cyan: '#06b6d4',
      white: '#f3f4f6',
    },
  })

  fitAddon = new FitAddon()
  term.loadAddon(fitAddon)
  term.open(terminalContainer.value)
  fitAddon.fit()

  // Listen for SSH data
  unlistenData = await listen('ssh-data', (event) => {
    const payload = event.payload
    if (typeof payload === 'object' && payload.session_id === props.sessionId) {
      term.write(payload.data)
    } else if (typeof payload === 'string') {
      term.write(payload)
    }
  })

  // Listen for SSH errors
  unlistenError = await listen('ssh-error', (event) => {
    const payload = event.payload
    if (typeof payload === 'object' && payload.session_id === props.sessionId) {
      term.writeln(`\r\n\x1b[31mError: ${payload.error}\x1b[0m`)
    }
  })

  // Listen for SSH disconnections
  unlistenDisconnected = await listen('ssh-disconnected', (event) => {
    if (event.payload === props.sessionId) {
      isDisconnected.value = true
    }
  })

  // Listen for SSH reconnections
  unlistenReconnected = await listen('ssh-reconnected', (event) => {
    if (event.payload === props.sessionId) {
      isDisconnected.value = false
      isReconnecting.value = false
      term.clear()
    }
  })

  // Send keystrokes to SSH
  term.onData((data) => {
    invoke('ssh_write', { sessionId: props.sessionId, data }).catch((err) => {
      console.error('ssh_write failed:', err)
    })
  })

  // Handle resize
  resizeObserver = new ResizeObserver(() => {
    if (fitAddon) {
      fitAddon.fit()
    }
  })
  resizeObserver.observe(terminalContainer.value)
})

async function reconnect() {
  isReconnecting.value = true
  try {
    await invoke('ssh_reconnect', { sessionId: props.sessionId })
  } catch (err) {
    console.error('Reconnect failed:', err)
    isReconnecting.value = false
  }
}

onUnmounted(() => {
  if (unlistenData) unlistenData()
  if (unlistenError) unlistenError()
  if (unlistenDisconnected) unlistenDisconnected()
  if (unlistenReconnected) unlistenReconnected()
  if (resizeObserver) resizeObserver.disconnect()
  if (term) term.dispose()
})

// Handle sessionId changes
watch(() => props.sessionId, (newId, oldId) => {
  if (term && newId !== oldId) {
    term.clear()
  }
})
</script>
