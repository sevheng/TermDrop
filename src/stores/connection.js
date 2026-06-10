import { defineStore } from 'pinia'
import { ref, shallowRef, computed, reactive } from 'vue'
import { invoke as tauriInvoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open, save } from '@tauri-apps/plugin-dialog'

const INVOKE_TIMEOUT_MS = 10000 // 10 seconds

/**
 * Wrap Tauri invoke() with a freeze-detection timer.
 * If the call takes longer than INVOKE_TIMEOUT_MS, a warning toast is shown.
 */
function invoke(cmd, args = {}) {
  const start = performance.now()
  let warned = false
  const timer = setTimeout(() => {
    warned = true
    window.dispatchEvent(new CustomEvent('app-toast', {
      detail: { message: `${cmd} is taking longer than expected...`, type: 'warning' }
    }))
  }, INVOKE_TIMEOUT_MS)

  return tauriInvoke(cmd, args).finally(() => {
    clearTimeout(timer)
    const elapsed = performance.now() - start
    if (elapsed > INVOKE_TIMEOUT_MS) {
      console.warn(`[SLOW] ${cmd} took ${elapsed.toFixed(0)}ms`, args)
    }
  })
}

export const useConnectionStore = defineStore('connection', () => {
  const hosts = ref([])
  const tabs = shallowRef([])
  const activeTabId = ref(null)
  const connectingHostId = ref(null)

  // Global SSH event router: one listener per event type, routes to terminal callbacks by sessionId
  const terminalHandlers = reactive(new Map())

  function registerTerminal(sessionId, handlers) {
    terminalHandlers.set(sessionId, handlers)
  }

  function unregisterTerminal(sessionId) {
    terminalHandlers.delete(sessionId)
  }

  // Register global listeners once (fire-and-forget, lifetime of app)
  listen('ssh-data', (event) => {
    const payload = event.payload
    if (typeof payload === 'object' && payload.session_id) {
      const handler = terminalHandlers.get(payload.session_id)
      if (handler && handler.write) handler.write(payload.data)
    }
  })

  listen('ssh-error', (event) => {
    const payload = event.payload
    if (typeof payload === 'object' && payload.session_id) {
      const handler = terminalHandlers.get(payload.session_id)
      if (handler && handler.writeError) handler.writeError(payload.error)
    }
  })

  listen('ssh-connected', (event) => {
    const sessionId = event.payload
    const handler = terminalHandlers.get(sessionId)
    if (handler && handler.onConnected) handler.onConnected()
  })

  listen('ssh-disconnected', (event) => {
    const sessionId = event.payload
    tabs.value = tabs.value.map(t => t.id === sessionId ? { ...t, connected: false } : t)
    const handler = terminalHandlers.get(sessionId)
    if (handler && handler.onDisconnected) handler.onDisconnected()
  })

  listen('ssh-reconnected', (event) => {
    const sessionId = event.payload
    tabs.value = tabs.value.map(t => t.id === sessionId ? { ...t, connected: true } : t)
    const handler = terminalHandlers.get(sessionId)
    if (handler && handler.onReconnected) handler.onReconnected()
  })
  const settings = ref({
    font_size: '14',
    download_path: '',
  })

  const systemStatus = ref(new Map())
  const prevNetStats = ref(new Map())

  function getSystemStatus(hostId) {
    return systemStatus.value.get(hostId) || null
  }

  function setSystemStatus(hostId, data) {
    systemStatus.value.set(hostId, { ...data, timestamp: Date.now() })
  }

  function getNetStats(hostId) {
    return prevNetStats.value.get(hostId) || { rx: 0, tx: 0, time: 0 }
  }

  function setNetStats(hostId, data) {
    prevNetStats.value.set(hostId, data)
  }

  const securityReports = ref(new Map())
  const securityReportVersion = ref(0)

  function getSecurityReport(hostId) {
    return securityReports.value.get(hostId) || null
  }

  function setSecurityLoading(hostId) {
    securityReports.value.set(hostId, { report: null, loading: true, error: null })
    securityReportVersion.value++
  }

  function setSecurityReport(hostId, report) {
    securityReports.value.set(hostId, { report, loading: false, error: null })
    securityReportVersion.value++
  }

  function setSecurityError(hostId, error) {
    securityReports.value.set(hostId, { report: null, loading: false, error })
    securityReportVersion.value++
  }

  async function runSecurityAudit(hostId) {
    setSecurityLoading(hostId)
    try {
      const report = await invoke('run_security_audit', { hostId })
      setSecurityReport(hostId, report)
    } catch (err) {
      setSecurityError(hostId, String(err))
    }
  }

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

  async function setHostGroup(id, group) {
    await invoke('update_host_group', { id, group })
    await loadHosts()
  }

  async function renameGroup(oldName, newName) {
    await invoke('batch_update_host_group', { oldGroup: oldName, newGroup: newName })
    await loadHosts()
  }

  async function deleteGroup(groupName) {
    await invoke('batch_clear_host_group', { group: groupName })
    await loadHosts()
  }

  async function setHostFavorite(id, favorite) {
    await invoke('update_host_favorite', { id, favorite: favorite ? 1 : 0 })
    await loadHosts()
  }

  async function setHostLastConnected(id) {
    await invoke('update_host_last_connected', { id })
  }

  async function exportHosts() {
    const json = await invoke('export_hosts')
    const filePath = await save({
      filters: [{ name: 'JSON', extensions: ['json'] }],
      defaultPath: `ssh-hosts-${new Date().toISOString().split('T')[0]}.json`,
    })
    if (filePath) {
      await invoke('write_file', { path: filePath, content: json })
    }
  }

  async function importHosts(fileContent) {
    const count = await invoke('import_hosts', { json: fileContent })
    await loadHosts()
    return count
  }

  async function connect(hostId, providedPassword = null) {
    const host = hosts.value.find(h => h.id === hostId)
    const isKeyAuth = host?.auth_type === 'key'
    connectingHostId.value = hostId

    // Estimate terminal size before creating PTY so the remote shell
    // starts with roughly the right dimensions instead of default 80x24.
    const estCols = Math.max(80, Math.floor((window.innerWidth - 48) / 8))
    const estRows = Math.max(24, Math.floor((window.innerHeight - 200) / 16))

    let sessionId
    const sshArgs = { hostId, cols: estCols, rows: estRows }
    if (!isKeyAuth && providedPassword) {
      sshArgs.password = providedPassword
    }
    try {
      sessionId = await invoke('ssh_connect', sshArgs)
    } catch (err) {
      connectingHostId.value = null
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

    // Track last connected
    setHostLastConnected(hostId).catch(() => {})

    const tab = {
      id: sessionId,
      sftpSessionId: null,
      hostId,
      name: host?.name || host?.host || 'Unknown',
      connected: true,
      connecting: true,
    }
    tabs.value = [...tabs.value, tab]
    activeTabId.value = sessionId

    // Try SFTP in background — don't block tab creation
    try {
      const sftpArgs = { hostId }
      if (!isKeyAuth && providedPassword) {
        sftpArgs.password = providedPassword
      }
      const sftpId = await invoke('sftp_connect', sftpArgs)
      tabs.value = tabs.value.map(t =>
        t.id === sessionId ? { ...t, sftpSessionId: sftpId, connecting: false } : t
      )
    } catch (err) {
      console.warn('SFTP connection failed:', err)
      window.dispatchEvent(new CustomEvent('app-toast', { detail: { message: 'SFTP connection failed: ' + err, type: 'warning' } }))
    }
    // Run security audit in background — don't block tab creation
    runSecurityAudit(hostId).catch(() => {})

    connectingHostId.value = null
    return sessionId
  }

  async function disconnect(sessionId) {
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

  // Port forward actions
  async function getPortForwards(hostId) {
    return await invoke('get_port_forwards', { hostId })
  }

  async function addPortForward(forward) {
    return await invoke('add_port_forward', { forward })
  }

  async function deletePortForward(id) {
    await invoke('delete_port_forward', { id })
  }

  async function startPortForward(ruleId) {
    await invoke('start_port_forward', { ruleId })
  }

  async function stopPortForward(ruleId) {
    await invoke('stop_port_forward', { ruleId })
  }

  async function getPortForwardStatus(ruleId) {
    return await invoke('get_port_forward_status', { ruleId })
  }

  async function loadSettings() {
    const [font_size, download_path] = await Promise.all([
      invoke('get_setting', { key: 'font_size' }),
      invoke('get_setting', { key: 'download_path' }),
    ])
    settings.value = {
      font_size: font_size || '14',
      download_path: download_path || '',
    }
    return settings.value
  }

  async function saveSettings(newSettings) {
    await Promise.all([
      invoke('set_setting', { key: 'font_size', value: String(newSettings.font_size || 14) }),
      invoke('set_setting', { key: 'download_path', value: newSettings.download_path || '' }),
    ])
    settings.value = { ...settings.value, ...newSettings }
  }

  return {
    hosts,
    tabs,
    activeTabId,
    activeTab,
    connectingHostId,
    loadHosts,
    addHost,
    updateHost,
    removeHost,
    storePassword,
    setHostGroup,
    renameGroup,
    deleteGroup,
    setHostFavorite,
    setHostLastConnected,
    exportHosts,
    importHosts,
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
    getPortForwards,
    addPortForward,
    deletePortForward,
    startPortForward,
    stopPortForward,
    getPortForwardStatus,
    settings,
    loadSettings,
    saveSettings,
    systemStatus,
    getSystemStatus,
    setSystemStatus,
    getNetStats,
    setNetStats,
    securityReports,
    securityReportVersion,
    getSecurityReport,
    runSecurityAudit,
    registerTerminal,
    unregisterTerminal,
  }
})