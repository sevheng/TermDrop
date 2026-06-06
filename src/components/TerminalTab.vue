<template>
  <div ref="terminalContainer" class="w-full h-full bg-gray-900"></div>
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
let resizeObserver = null

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

onUnmounted(() => {
  if (unlistenData) unlistenData()
  if (unlistenError) unlistenError()
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
