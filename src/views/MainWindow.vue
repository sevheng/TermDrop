<template>
  <div class="flex h-screen bg-gray-900 text-white" :class="{ 'light-theme': currentTheme === 'light' }">
    <HostSidebar />
    
    <div class="flex-1 flex flex-col min-w-0">
      <!-- Header with tabs and settings -->
      <div class="flex border-b border-gray-700 bg-gray-800 items-center justify-between">
        <div class="flex">
          <button
            v-for="tab in store.tabs"
            :key="tab.id"
            @click="store.setActiveTab(tab.id)"
            class="px-4 py-2 text-sm border-r border-gray-700 flex items-center gap-2"
            :class="tab.id === store.activeTabId ? 'bg-gray-700 text-white' : 'text-gray-400 hover:text-gray-200'"
          >
            <span>{{ tab.name }}</span>
            <span
              @click.stop="store.disconnect(tab.id)"
              class="hover:text-red-400 cursor-pointer ml-1"
            >×</span>
          </button>
        </div>
        <button @click="showSettings = true" class="px-3 py-2 text-gray-400 hover:text-white">
          <Settings :size="16" />
        </button>
      </div>
      
      <!-- Terminal + SFTP area -->
      <div class="flex-1 flex overflow-hidden">
        <div class="flex-1 relative">
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
        
        <SftpPanel
          v-if="store.activeTab?.sftpSessionId"
          :sftpSessionId="store.activeTab.sftpSessionId"
          class="w-80 border-l border-gray-700 shrink-0"
        />
      </div>
    </div>
    
    <SettingsPanel
      :show="showSettings"
      @close="showSettings = false"
      @saved="onSettingsSaved"
    />
  </div>
</template>

<script setup>
import { ref } from 'vue'
import HostSidebar from '../components/HostSidebar.vue'
import TerminalTab from '../components/TerminalTab.vue'
import SftpPanel from '../components/SftpPanel.vue'
import SettingsPanel from '../components/SettingsPanel.vue'
import { useConnectionStore } from '../stores/connection.js'
import { Terminal as TerminalIcon, Settings } from 'lucide-vue-next'

const store = useConnectionStore()
const showSettings = ref(false)
const currentTheme = ref('dark')

function onSettingsSaved(settings) {
  currentTheme.value = settings.theme
  document.documentElement.classList.toggle('light-theme', settings.theme === 'light')
}
</script>

<style>
.light-theme {
  --bg-primary: #f3f4f6;
  --bg-secondary: #e5e7eb;
  --text-primary: #111827;
  --text-secondary: #374151;
}
</style>
