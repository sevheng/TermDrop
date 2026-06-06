<template>
  <div class="flex h-screen bg-gray-900 text-white">
    <HostSidebar />
    
    <div class="flex-1 flex flex-col min-w-0">
      <!-- Tab bar -->
      <div v-if="store.tabs.length > 0" class="flex border-b border-gray-700 bg-gray-800">
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
      
      <!-- Terminal + SFTP area -->
      <div class="flex-1 flex overflow-hidden">
        <!-- Terminal tabs — v-show preserves xterm instances -->
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
        
        <!-- SFTP Panel -->
        <SftpPanel
          v-if="store.activeTab?.sftpSessionId"
          :sftpSessionId="store.activeTab.sftpSessionId"
          class="w-80 border-l border-gray-700 shrink-0"
        />
      </div>
    </div>
  </div>
</template>

<script setup>
import HostSidebar from '../components/HostSidebar.vue'
import TerminalTab from '../components/TerminalTab.vue'
import SftpPanel from '../components/SftpPanel.vue'
import { useConnectionStore } from '../stores/connection.js'
import { Terminal as TerminalIcon } from 'lucide-vue-next'

const store = useConnectionStore()
</script>
