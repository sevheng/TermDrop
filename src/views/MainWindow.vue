<template>
  <div
    class="flex h-screen bg-gray-50 text-gray-900 dark:bg-gray-900 dark:text-white transition-colors"
    :class="{ 'light-theme': currentTheme === 'light' }"
  >
    <!-- Host Sidebar -->
    <div
      class="h-full bg-white border-r border-gray-200 flex flex-col shrink-0 dark:bg-gray-800 dark:border-gray-700"
      :style="{ width: sidebarWidth + 'px' }"
    >
      <HostSidebar />
    </div>

    <!-- Sidebar resize handle -->
    <div
      class="w-1.5 shrink-0 cursor-col-resize bg-gray-300 hover:bg-blue-500 transition-colors z-10 dark:bg-gray-700"
      @mousedown="startResizeSidebar"
    ></div>

    <div class="flex-1 flex flex-col min-w-0">
      <!-- Header with tabs and settings -->
      <div class="flex border-b border-gray-200 bg-white items-center justify-between dark:border-gray-700 dark:bg-gray-800">
        <div class="flex overflow-x-auto">
          <button
            v-for="tab in store.tabs"
            :key="tab.id"
            @click="store.setActiveTab(tab.id)"
            class="px-3 py-1.5 text-xs border-r border-gray-200 flex items-center gap-1.5 whitespace-nowrap transition-colors dark:border-gray-700"
            :class="tab.id === store.activeTabId
              ? 'bg-gray-200 text-gray-900 dark:bg-gray-700 dark:text-white'
              : 'text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200'"
          >
            <span
              class="w-2 h-2 rounded-full shrink-0"
              :class="tab.connected !== false ? 'bg-green-500' : 'bg-red-500'"
            ></span>
            <span>{{ tab.name }}</span>
            <span
              v-if="tab.connecting"
              class="w-3 h-3 border-2 border-blue-500 border-t-transparent rounded-full animate-spin shrink-0"
            ></span>
            <span
              v-else
              @click.stop="confirmDisconnect(tab.id, tab.name)"
              class="hover:text-red-400 cursor-pointer ml-1"
            >
              <X :size="14" />
            </span>
          </button>
        </div>
        <div class="flex items-center shrink-0">
          <button @click="showShortcuts = true" class="px-2 py-1.5 text-gray-500 hover:text-gray-900 dark:text-gray-400 dark:hover:text-white" title="Keyboard shortcuts">
            <Keyboard :size="14" />
          </button>
          <button @click="showSettings = true" class="px-2 py-1.5 text-gray-500 hover:text-gray-900 dark:text-gray-400 dark:hover:text-white" title="Settings">
            <Settings :size="14" />
          </button>
        </div>
      </div>

      <!-- Terminal + SFTP area -->
      <div class="flex-1 flex overflow-hidden">
        <div class="flex-1 relative min-w-0">
          <KeepAlive :max="5">
            <TerminalTab
              v-if="store.activeTab"
              :key="store.activeTab.id"
              :sessionId="store.activeTab.id"
              :hostId="store.activeTab.hostId"
              class="w-full h-full"
            />
          </KeepAlive>
          <div
            v-if="!store.activeTabId"
            class="flex items-center justify-center h-full text-gray-400 dark:text-gray-500"
          >
            <div class="text-center">
              <TerminalIcon :size="40" class="mx-auto mb-3 opacity-50" />
              <p class="text-base">Select a host to connect</p>
            </div>
          </div>
        </div>

        <!-- SFTP panel resize handle -->
        <div
          v-if="store.activeTab"
          class="w-1.5 shrink-0 cursor-col-resize bg-gray-300 hover:bg-blue-500 transition-colors z-10 dark:bg-gray-700"
          @mousedown="startResizeSftp"
        ></div>

        <div
          v-if="store.activeTab"
          class="border-l border-gray-200 shrink-0 bg-white flex flex-col dark:border-gray-700 dark:bg-gray-800"
          :style="{ width: sftpWidth + 'px' }"
        >
          <!-- Panel tabs -->
          <div class="flex border-b border-gray-200 dark:border-gray-700">
            <button
              @click="showPortForwards = false"
              class="flex-1 py-1.5 text-xs font-medium transition-colors relative"
              :class="!showPortForwards
                ? 'text-blue-600 dark:text-blue-400'
                : 'text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200'"
            >
              SFTP
              <span
                v-if="!showPortForwards"
                class="absolute bottom-0 left-2 right-2 h-0.5 bg-blue-600 rounded-full dark:bg-blue-400"
              />
            </button>
            <button
              @click="showPortForwards = true"
              class="flex-1 py-1.5 text-xs font-medium transition-colors relative"
              :class="showPortForwards
                ? 'text-blue-600 dark:text-blue-400'
                : 'text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200'"
            >
              Tunnels
              <span
                v-if="showPortForwards"
                class="absolute bottom-0 left-2 right-2 h-0.5 bg-blue-600 rounded-full dark:bg-blue-400"
              />
            </button>
          </div>

          <div class="flex-1 overflow-hidden">
            <PortForwardPanel
              v-if="showPortForwards"
              :hostId="store.activeTab.hostId"
              @add="showForwardModal = true"
              class="w-full h-full"
            />
            <template v-else>
              <SftpPanel
                v-if="store.activeTab.sftpSessionId"
                :sftpSessionId="store.activeTab.sftpSessionId"
                class="w-full h-full"
              />
              <div v-else class="flex-1 flex flex-col items-center justify-center text-gray-400 dark:text-gray-500 h-full">
                <Loader2 :size="24" class="animate-spin mb-2" />
                <span class="text-sm">Connecting SFTP...</span>
              </div>
            </template>
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

    <PortForwardModal
      :show="showForwardModal"
      :hostId="store.activeTab?.hostId"
      @close="showForwardModal = false"
      @save="onForwardSaved"
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
import { ref, onMounted, onUnmounted, defineAsyncComponent } from 'vue'
import HostSidebar from '../components/HostSidebar.vue'
import TerminalTab from '../components/TerminalTab.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import { useConnectionStore } from '../stores/connection.js'
import { Terminal as TerminalIcon, Settings, Loader2, Keyboard, X } from 'lucide-vue-next'
import { invoke } from '@tauri-apps/api/core'

const SftpPanel = defineAsyncComponent(() => import('../components/SftpPanel.vue'))
const PortForwardPanel = defineAsyncComponent(() => import('../components/PortForwardPanel.vue'))
const PortForwardModal = defineAsyncComponent(() => import('../components/PortForwardModal.vue'))
const SettingsPanel = defineAsyncComponent(() => import('../components/SettingsPanel.vue'))
const ShortcutsHelp = defineAsyncComponent(() => import('../components/ShortcutsHelp.vue'))

const store = useConnectionStore()
const showSettings = ref(false)
const showShortcuts = ref(false)
const showPortForwards = ref(false)
const showForwardModal = ref(false)
const currentTheme = ref('dark')
const sidebarWidth = ref(220)
const sftpWidth = ref(260)

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
  document.documentElement.classList.toggle('dark', settings.theme === 'dark')
  document.documentElement.classList.toggle('light-theme', settings.theme === 'light')
}

async function onForwardSaved(forwardData) {
  try {
    await store.addPortForward(forwardData)
    showForwardModal.value = false
    window.dispatchEvent(new CustomEvent('app-toast', { detail: { message: 'Port forward added', type: 'success' } }))
    // Refresh the panel if it's open
    if (showPortForwards.value) {
      // The panel will auto-refresh via its watcher
    }
  } catch (err) {
    window.dispatchEvent(new CustomEvent('app-toast', { detail: { message: 'Failed to add: ' + err, type: 'error' } }))
  }
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

onMounted(async () => {
  const savedSidebarWidth = localStorage.getItem('sidebar-width')
  if (savedSidebarWidth) sidebarWidth.value = parseInt(savedSidebarWidth)
  const savedSftpWidth = localStorage.getItem('sftp-width')
  if (savedSftpWidth) sftpWidth.value = parseInt(savedSftpWidth)

  const savedTheme = await invoke('get_setting', { key: 'theme' })
  const theme = savedTheme || 'dark'
  currentTheme.value = theme
  document.documentElement.classList.toggle('dark', theme === 'dark')
  document.documentElement.classList.toggle('light-theme', theme === 'light')

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
