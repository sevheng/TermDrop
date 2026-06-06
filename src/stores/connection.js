import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

export const useConnectionStore = defineStore('connection', () => {
  const hosts = ref([])
  const tabs = ref([])
  const activeTabId = ref(null)

  const activeTab = computed(() => {
    return tabs.value.find(t => t.id === activeTabId.value)
  })

  async function loadHosts() {
    hosts.value = await invoke('get_hosts')
  }

  async function addHost(host) {
    const id = await invoke('add_host', { host })
    await loadHosts()
    return id
  }

  async function updateHost(id, host) {
    await invoke('update_host', { id, host })
    await loadHosts()
  }

  async function removeHost(id) {
    await invoke('delete_host', { id })
    await loadHosts()
  }

  async function storePassword(hostId, password) {
    await invoke('store_password', { hostId, password })
  }

  async function connect(hostId) {
    const sessionId = await invoke('ssh_connect', { hostId })
    const host = hosts.value.find(h => h.id === hostId)
    tabs.value.push({
      id: sessionId,
      hostId,
      name: host?.name || host?.host || 'Unknown',
    })
    activeTabId.value = sessionId

    // Listen for disconnect
    const unlisten = await listen('ssh-disconnected', (event) => {
      if (event.payload === sessionId) {
        tabs.value = tabs.value.filter(t => t.id !== sessionId)
        if (activeTabId.value === sessionId) {
          activeTabId.value = tabs.value.length > 0 ? tabs.value[0].id : null
        }
        unlisten()
      }
    })

    return sessionId
  }

  async function disconnect(sessionId) {
    await invoke('ssh_disconnect', { sessionId })
  }

  async function writeData(sessionId, data) {
    await invoke('ssh_write', { sessionId, data })
  }

  function setActiveTab(sessionId) {
    activeTabId.value = sessionId
  }

  return {
    hosts,
    tabs,
    activeTabId,
    activeTab,
    loadHosts,
    addHost,
    updateHost,
    removeHost,
    storePassword,
    connect,
    disconnect,
    writeData,
    setActiveTab,
  }
})
