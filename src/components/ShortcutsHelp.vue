<template>
  <div
    v-if="show"
    class="fixed inset-0 bg-black/60 flex items-center justify-center z-[100]"
    @click.self="onClose"
  >
    <div class="bg-gray-800 rounded-lg p-5 w-[28rem] max-w-[90vw] border border-gray-700 shadow-xl max-h-[80vh] flex flex-col">
      <div class="flex items-center justify-between mb-4">
        <h3 class="text-base font-semibold text-white">Keyboard Shortcuts</h3>
        <button @click="onClose" class="text-gray-400 hover:text-white">×</button>
      </div>

      <div class="overflow-y-auto space-y-4 pr-1">
        <section>
          <h4 class="text-xs font-semibold text-gray-400 uppercase tracking-wider mb-2">Terminal</h4>
          <div class="space-y-1">
            <div v-for="s in terminalShortcuts" :key="s.keys" class="flex justify-between text-sm">
              <span class="text-gray-300">{{ s.label }}</span>
              <kbd class="bg-gray-700 px-1.5 py-0.5 rounded text-xs text-gray-200 font-mono">{{ s.keys }}</kbd>
            </div>
          </div>
        </section>

        <section>
          <h4 class="text-xs font-semibold text-gray-400 uppercase tracking-wider mb-2">Tabs</h4>
          <div class="space-y-1">
            <div v-for="s in tabShortcuts" :key="s.keys" class="flex justify-between text-sm">
              <span class="text-gray-300">{{ s.label }}</span>
              <kbd class="bg-gray-700 px-1.5 py-0.5 rounded text-xs text-gray-200 font-mono">{{ s.keys }}</kbd>
            </div>
          </div>
        </section>

        <section>
          <h4 class="text-xs font-semibold text-gray-400 uppercase tracking-wider mb-2">SFTP</h4>
          <div class="space-y-1">
            <div v-for="s in sftpShortcuts" :key="s.keys" class="flex justify-between text-sm">
              <span class="text-gray-300">{{ s.label }}</span>
              <kbd class="bg-gray-700 px-1.5 py-0.5 rounded text-xs text-gray-200 font-mono">{{ s.keys }}</kbd>
            </div>
          </div>
        </section>
      </div>

      <div class="mt-4 pt-3 border-t border-gray-700 text-right">
        <button
          @click="onClose"
          class="px-3 py-1.5 text-sm bg-blue-600 hover:bg-blue-700 text-white rounded transition-colors"
        >
          Close
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
const props = defineProps({
  show: Boolean,
})

const emit = defineEmits(['close'])

function onClose() {
  emit('close')
}

const terminalShortcuts = [
  { keys: 'Ctrl + Shift + C', label: 'Copy selection' },
  { keys: 'Ctrl + Shift + V', label: 'Paste from clipboard' },
  { keys: 'Ctrl + Shift + A', label: 'Select all' },
  { keys: 'Ctrl + F', label: 'Find in terminal' },
]

const tabShortcuts = [
  { keys: 'Ctrl + Tab', label: 'Next tab' },
  { keys: 'Ctrl + Shift + Tab', label: 'Previous tab' },
  { keys: 'Ctrl + W', label: 'Close active tab' },
]

const sftpShortcuts = [
  { keys: 'Double-click', label: 'Open folder' },
  { keys: 'Right-click', label: 'File actions' },
  { keys: 'Drag & drop', label: 'Upload files' },
]
</script>
