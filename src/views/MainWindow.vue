<template>
  <div class="flex h-screen bg-gray-900 text-white" :class="{ 'light-theme': currentTheme === 'light' }">
    <!-- Host Sidebar -->
    <div
      class="h-full bg-gray-800 border-r border-gray-700 flex flex-col shrink-0"
      :style="{ width: sidebarWidth + 'px' }"
    >
      <HostSidebar />
    </div>

    <!-- Sidebar resize handle -->
    <div
      class="w-1.5 shrink-0 cursor-col-resize bg-gray-700 hover:bg-blue-500 transition-colors z-10"
      @mousedown="startResizeSidebar"
    ></div>

    <div class="flex-1 flex flex-col min-w-0">
      <!-- Header with tabs and settings -->
      <div class="flex border-b border-gray-700 bg-gray-800 items-center justify-between">
        <div class="flex overflow-x-auto">
          <button
            v-for="tab in store.tabs"
            :key="tab.id"
            @click="store.setActiveTab(tab.id)"
            class="px-4 py-2 text-sm border-r border-gray-700 flex items-center gap-2 whitespace-nowrap"
            :class="tab.id === store.activeTabId ? 'bg-gray-700 text-white' : 'text-gray-400 hover:text-gray-200'"
          >
            <span
              class="w-2 h-2 rounded-full shrink-0"
              :class="tab.connected !== false ? 'bg-green-500' : 'bg-red-500'"
            ></span>
            <span>{{ tab.name }}</span>
            <span
              @click.stop="confirmDisconnect(tab.id, tab.name)"
              class="hover:text-red-400 cursor-pointer ml-1"
            >×</span>
          </button>
        </div>
        <div class="flex items-center shrink-0">
          <button @click="showShortcuts = true" class="px-3 py-2 text-gray-400 hover:text-white" title="Keyboard shortcuts">
            <Keyboard :size="16" />
          </button>
          <button @click="showSettings = true" class="px-3 py-2 text-gray-400 hover:text-white" title="Settings">
            <Settings :size="16" />
          </button>
        </div>
      </div>

      <!-- Terminal + SFTP area -->
      <div class="flex-1 flex overflow-hidden">
        <div class="flex-1 relative min-w-0">
          <TerminalTab
            v-for="tab in store.tabs"
            :key="tab.id"
            :sessionId="tab.id"
            :class="tab.id === store.activeTabId ? 'block' : 'hidden'"
            class="w-full h-full"
          />
          <div
            v-if="!store.activeTabId"
            class="flex items-center justify-center h-full text-gray-500"
          >
            <div class="text-center">
              <TerminalIcon :size="48" class="mx-auto mb-4 opacity-50" />
              <p class="text-lg">Select a host to connect</p>
            </div>
          </div>
        </div>

        <!-- SFTP panel resize handle -->
        <div
          v-if="store.activeTab"
          class="w-1.5 shrink-0 cursor-col-resize bg-gray-700 hover:bg-blue-500 transition-colors z-10"
          @mousedown="startResizeSftp"
        ></div>

        <div
          v-if="store.activeTab"
          class="border-l border-gray-700 shrink-0 bg-gray-800 flex flex-col"
          :style="{ width: sftpWidth + 'px' }"
        >
          <SftpPanel
            v-if="store.activeTab.sftpSessionId"
            :sftpSessionId="store.activeTab.sftpSessionId"
            class="w-full h-full"
          />
          <div v-else class="flex-1 flex flex-col items-center justify-center text-gray-500">
            <Loader2 :size="24" class="animate-spin mb-2" />
            <span class="text-sm">Connecting SFTP...</span>
          </div>
        </div>
      </div>
    </div>

    <SettingsPanel
      :show="showSettings"
      @close="showSettings = false"
      @saved="onSettingsSaved"
    />

    <ShortcutsHelp
      :show="showShortcuts"
      @close="showShortcuts = false"
    />

    <ConfirmDialog
      :show="confirmDialog.show"
      :title="confirmDialog.title"
      :message="confirmDialog.message"
      :danger="confirmDialog.danger"
      confirm-text="Close"
      @confirm="confirmDialog.onConfirm"
      @cancel="confirmDialog.show = false"
    />
  </div>
</template>

<script setup>
import { ref, onMounted, onUnmounted } from 'vue'
import HostSidebar from '../components/HostSidebar.vue'
import TerminalTab from '../components/TerminalTab.vue'
import SftpPanel from '../components/SftpPanel.vue'
import SettingsPanel from '../components/SettingsPanel.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import ShortcutsHelp from '../components/ShortcutsHelp.vue'
import { useConnectionStore } from '../stores/connection.js'
import { Terminal as TerminalIcon, Settings, Loader2, Keyboard } from 'lucide-vue-next'

const store = useConnectionStore()
const showSettings = ref(false)
const showShortcuts = ref(false)
const currentTheme = ref('dark')
const sidebarWidth = ref(256)
const sftpWidth = ref(280)

const confirmDialog = ref({
  show: false,
  title: '',
  message: '',
  danger: false,
  onConfirm: () => {},
})

function openConfirm(options) {
  confirmDialog.value = {
    show: true,
    title: options.title || 'Confirm',
    message: options.message || '',
    danger: options.danger || false,
    onConfirm: () => {
      confirmDialog.value.show = false
      options.onConfirm()
    },
  }
}

function confirmDisconnect(sessionId, name) {
  openConfirm({
    title: 'Close Session',
    message: `Close session "${name}"?`,
    danger: true,
    onConfirm: () => {
      store.disconnect(sessionId)
    },
  })
}

function onSettingsSaved(settings) {
  currentTheme.value = settings.theme
  document.documentElement.classList.toggle('light-theme', settings.theme === 'light')
}

// Global keyboard shortcuts
function onKeyDown(e) {
  if (store.tabs.length === 0) return

  // Ctrl+Tab / Ctrl+PageDown → next tab
  if (e.ctrlKey && !e.shiftKey && (e.key === 'Tab' || e.key === 'PageDown')) {
    e.preventDefault()
    const idx = store.tabs.findIndex(t => t.id === store.activeTabId)
    const nextIdx = (idx + 1) % store.tabs.length
    store.setActiveTab(store.tabs[nextIdx].id)
    return
  }

  // Ctrl+Shift+Tab / Ctrl+PageUp → previous tab
  if (e.ctrlKey && e.shiftKey && (e.key === 'Tab' || e.key === 'PageUp')) {
    e.preventDefault()
    const idx = store.tabs.findIndex(t => t.id === store.activeTabId)
    const prevIdx = (idx - 1 + store.tabs.length) % store.tabs.length
    store.setActiveTab(store.tabs[prevIdx].id)
    return
  }

  // Ctrl+W → close active tab
  if (e.ctrlKey && e.key.toLowerCase() === 'w') {
    e.preventDefault()
    if (store.activeTabId) {
      const tab = store.tabs.find(t => t.id === store.activeTabId)
      if (tab) {
        confirmDisconnect(tab.id, tab.name)
      }
    }
    return
  }

  // Ctrl+Shift+/ → show shortcuts help
  if (e.ctrlKey && e.shiftKey && e.key === '?') {
    e.preventDefault()
    showShortcuts.value = true
    return
  }
}

// Resizable sidebar
function startResizeSidebar(e) {
  e.preventDefault()
  const startX = e.clientX
  const startWidth = sidebarWidth.value

  function onMove(moveEvent) {
    const delta = moveEvent.clientX - startX
    sidebarWidth.value = Math.max(150, Math.min(400, startWidth + delta))
  }

  function onUp() {
    window.removeEventListener('mousemove', onMove)
    window.removeEventListener('mouseup', onUp)
    localStorage.setItem('sidebar-width', String(sidebarWidth.value))
  }

  window.addEventListener('mousemove', onMove)
  window.addEventListener('mouseup', onUp)
}

// Resizable SFTP panel
function startResizeSftp(e) {
  e.preventDefault()
  const startX = e.clientX
  const startWidth = sftpWidth.value

  function onMove(moveEvent) {
    // SFTP panel is on the right, so dragging left increases width
    const delta = startX - moveEvent.clientX
    sftpWidth.value = Math.max(180, Math.min(420, startWidth + delta))
  }

  function onUp() {
    window.removeEventListener('mousemove', onMove)
    window.removeEventListener('mouseup', onUp)
    localStorage.setItem('sftp-width', String(sftpWidth.value))
  }

  window.addEventListener('mousemove', onMove)
  window.addEventListener('mouseup', onUp)
}

onMounted(() => {
  const savedSidebarWidth = localStorage.getItem('sidebar-width')
  if (savedSidebarWidth) sidebarWidth.value = parseInt(savedSidebarWidth)
  const savedSftpWidth = localStorage.getItem('sftp-width')
  if (savedSftpWidth) sftpWidth.value = parseInt(savedSftpWidth)

  window.addEventListener('keydown', onKeyDown)
})

onUnmounted(() => {
  window.removeEventListener('keydown', onKeyDown)
})
</script>

<style>
.light-theme {
  --bg-primary: #f3f4f6;
  --bg-secondary: #e5e7eb;
  --text-primary: #111827;
  --text-secondary: #374151;
}
</style>
