import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'

export const useConnectionStore = defineStore('connection', () => {
  const hosts = ref([])
  const tabs = ref([])
  const activeTabId = ref(null)
  const tabListeners = ref(new Map())

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

  async function connect(hostId, providedPassword = null) {
    const host = hosts.value.find(h => h.id === hostId)
    const isKeyAuth = host?.auth_type === 'key'

    let sessionId
    const sshArgs = { hostId }
    if (!isKeyAuth && providedPassword) {
      sshArgs.password = providedPassword
    }
    try {
      sessionId = await invoke('ssh_connect', sshArgs)
    } catch (err) {
      const errStr = String(err)
      if (!isKeyAuth && (errStr.includes('keyring retrieve failed') || errStr.includes('No matching entry')) && !providedPassword) {
        const password = window.prompt('Password not found in keyring. Enter password for this host:')
        if (password) {
          await storePassword(hostId, password).catch(() => {})
          return connect(hostId, password)
        }
      }
      window.dispatchEvent(new CustomEvent('app-toast', { detail: { message: 'SSH connection failed: ' + err, type: 'error' } }))
      throw err
    }

    const tab = {
      id: sessionId,
      sftpSessionId: null,
      hostId,
      name: host?.name || host?.host || 'Unknown',
      connected: true,
    }
    tabs.value.push(tab)
    activeTabId.value = sessionId

    // Try SFTP in background — don't block tab creation
    try {
      const sftpArgs = { hostId }
      if (!isKeyAuth && providedPassword) {
        sftpArgs.password = providedPassword
      }
      const sftpId = await invoke('sftp_connect', sftpArgs)
      const idx = tabs.value.findIndex(t => t.id === sessionId)
      if (idx !== -1) {
        tabs.value[idx] = { ...tabs.value[idx], sftpSessionId: sftpId }
      }
    } catch (err) {
      console.warn('SFTP connection failed:', err)
      window.dispatchEvent(new CustomEvent('app-toast', { detail: { message: 'SFTP connection failed: ' + err, type: 'warning' } }))
    }

    const unlistenDisconnect = await listen('ssh-disconnected', (event) => {
      if (event.payload === sessionId) {
        const idx = tabs.value.findIndex(t => t.id === sessionId)
        if (idx !== -1) {
          tabs.value[idx] = { ...tabs.value[idx], connected: false }
        }
      }
    })

    const unlistenReconnected = await listen('ssh-reconnected', (event) => {
      if (event.payload === sessionId) {
        const idx = tabs.value.findIndex(t => t.id === sessionId)
        if (idx !== -1) {
          tabs.value[idx] = { ...tabs.value[idx], connected: true }
        }
      }
    })

    tabListeners.value.set(sessionId, {
      disconnect: unlistenDisconnect,
      reconnected: unlistenReconnected,
    })

    return sessionId
  }

  async function disconnect(sessionId) {
    const listeners = tabListeners.value.get(sessionId)
    if (listeners) {
      listeners.disconnect()
      listeners.reconnected()
      tabListeners.value.delete(sessionId)
    }

    const tab = tabs.value.find(t => t.id === sessionId)
    if (tab) {
      await invoke('sftp_disconnect', { sftpSessionId: tab.sftpSessionId }).catch(() => {})
    }
    await invoke('ssh_disconnect', { sessionId }).catch(() => {})
    tabs.value = tabs.value.filter(t => t.id !== sessionId)
    if (activeTabId.value === sessionId) {
      activeTabId.value = tabs.value.length > 0 ? tabs.value[0].id : null
    }
  }

  async function writeData(sessionId, data) {
    await invoke('ssh_write', { sessionId, data })
  }

  function setActiveTab(sessionId) {
    activeTabId.value = sessionId
  }

  // SFTP actions
  async function sftpList(sftpSessionId, path) {
    return await invoke('sftp_list', { sftpSessionId, path })
  }

  async function sftpUpload(sftpSessionId, remotePath) {
    const selected = await open({
      multiple: false,
      directory: false,
    })
    if (!selected) return null
    const localPath = Array.isArray(selected) ? selected[0] : selected
    const fileName = localPath.split(/[\\/]/).pop()
    const fullRemotePath = remotePath ? `${remotePath}/${fileName}` : fileName
    await invoke('sftp_upload', { sftpSessionId, localPath, remotePath: fullRemotePath })
    return localPath
  }

  async function sftpDownload(sftpSessionId, remotePath) {
    return await invoke('sftp_download', { sftpSessionId, remotePath })
  }

  async function sftpDelete(sftpSessionId, remotePath) {
    await invoke('sftp_delete', { sftpSessionId, remotePath })
  }

  async function sftpRename(sftpSessionId, oldPath, newPath) {
    await invoke('sftp_rename', { sftpSessionId, oldPath, newPath })
  }

  async function sftpMkdir(sftpSessionId, remotePath) {
    await invoke('sftp_mkdir', { sftpSessionId, remotePath })
  }

  async function sftpRmdir(sftpSessionId, remotePath) {
    await invoke('sftp_rmdir', { sftpSessionId, remotePath })
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
    sftpList,
    sftpUpload,
    sftpDownload,
    sftpDelete,
    sftpRename,
    sftpMkdir,
    sftpRmdir,
  }
})