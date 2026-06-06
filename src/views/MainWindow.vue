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
      
      <!-- Terminal area -->
      <div class="flex-1 relative">
        <TerminalTab
          v-if="store.activeTabId"
          :key="store.activeTabId"
          :sessionId="store.activeTabId"
        />
        <div
          v-else
          class="flex items-center justify-center h-full text-gray-500"
        >
          <div class="text-center">
            <TerminalIcon :size="48" class="mx-auto mb-4 opacity-50" />
            <p class="text-lg">Select a host to connect</p>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import HostSidebar from '../components/HostSidebar.vue'
import TerminalTab from '../components/TerminalTab.vue'
import { useConnectionStore } from '../stores/connection.js'
import { Terminal as TerminalIcon } from 'lucide-vue-next'

const store = useConnectionStore()
</script>
