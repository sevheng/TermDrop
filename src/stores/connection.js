import { defineStore } from 'pinia'
import { ref, shallowRef, computed, reactive } from 'vue'
import { invokeWithSlowWarning as invoke } from '../utils/invoke.js'
import { listen } from '@tauri-apps/api/event'
import { open, save } from '@tauri-apps/plugin-dialog'
import { toast } from '../utils/toast.js'
import { isMissingKeyringPassword } from '../utils/secretPrompt.js'
import { showPromptDialog } from '../composables/usePromptDialog.js'
import { TAB_KIND } from '../utils/tabKinds.js'
import { applyTheme } from '../composables/useTheme.js'
import { loadTerminalTab } from '../components/terminalTabLoader.js'



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
    theme: 'dark',
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

  function getSecurityReport(hostId) {
    return securityReports.value.get(hostId) || null
  }

  function setSecurityLoading(hostId) {
    securityReports.value.set(hostId, { report: null, loading: true, error: null })
  }

  function setSecurityReport(hostId, report) {
    securityReports.value.set(hostId, { report, loading: false, error: null })
  }

  function setSecurityError(hostId, error) {
    securityReports.value.set(hostId, { report: null, loading: false, error })
  }

  async function runSecurityAudit(hostId, force = false) {
    setSecurityLoading(hostId)
    try {
      const report = await invoke('run_security_audit', { hostId, force })
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

  /**
   * The settings and hosts loads that every launch needs, as one memoised
   * promise.
   *
   * Both used to run fire-and-forget from two different components' onMounted
   * hooks, so nothing could observe when the app actually had content -- which
   * the launch splash needs in order to know when to get out of the way.
   * Memoising is what keeps `get_hosts` to a single call however many places
   * ask for it.
   */
  let bootstrapped = null
  function bootstrap() {
    if (!bootstrapped) {
      // allSettled, not all: a failed settings read must not leave the app
      // looking like it never finished starting. Failures are logged rather
      // than toasted -- an empty sidebar is the visible symptom, and a toast
      // over the fade-in was noise.
      bootstrapped = Promise.allSettled([loadSettings(), loadHosts()]).then(results => {
        for (const r of results) {
          if (r.status === 'rejected') console.error('[bootstrap]', r.reason)
        }
        return results
      })
    }
    return bootstrapped
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

  /**
   * Import prepared entries and return the backend's summary. The reload runs
   * even when the call throws, because a partial import must still show up in
   * the sidebar rather than looking like nothing happened.
   */
  async function importHosts(entries) {
    try {
      return await invoke('import_hosts', { entries })
    } finally {
      await loadHosts()
    }
  }

  /**
   * Estimate terminal size before creating the PTY so the remote shell
   * starts with roughly the right dimensions instead of default 80x24.
   */
  function estimateTerminalSize() {
    return {
      cols: Math.max(80, Math.floor((window.innerWidth - 48) / 8)),
      rows: Math.max(24, Math.floor((window.innerHeight - 200) / 16)),
    }
  }

  /** Open the SFTP side channel for a tab; failure only warns, the tab stays. */
  async function attachSftp(sessionId, hostId, isKeyAuth, providedPassword) {
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
      toast('SFTP connection failed: ' + err, 'warning')
    }
  }

  async function connect(hostId, providedPassword = null) {
    // Start the terminal chunk now so it loads against the SSH handshake
    // rather than against the tab's first render. Idempotent -- the idle
    // warm-up in MainWindow has usually already done it.
    loadTerminalTab()

    const host = hosts.value.find(h => h.id === hostId)
    const isKeyAuth = host?.auth_type === 'key'
    connectingHostId.value = hostId

    let sessionId
    const sshArgs = { hostId, ...estimateTerminalSize() }
    if (!isKeyAuth && providedPassword) {
      sshArgs.password = providedPassword
    }
    try {
      sessionId = await invoke('ssh_connect', sshArgs)
    } catch (err) {
      connectingHostId.value = null
      if (!isKeyAuth && isMissingKeyringPassword(err) && !providedPassword) {
        const password = await showPromptDialog(
          'Password required',
          'Password not found in keyring. Enter password for this host:',
          '',
          'password',
        )
        if (password) {
          await storePassword(hostId, password).catch(() => {})
          return connect(hostId, password)
        }
      }
      toast('SSH connection failed: ' + err, 'error')
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
    await attachSftp(sessionId, hostId, isKeyAuth, providedPassword)
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
    // The backend drops its cached report when the last tab for a host goes
    // away; mirror that so a stale report is never shown as current.
    if (tab && !tabs.value.some(t => t.hostId === tab.hostId)) {
      securityReports.value.delete(tab.hostId)
    }
  }

  /**
   * What each datastore tab kind needs: which host field must be set for the
   * tab to be openable, what its ids look like, and what to tell the backend
   * when the last one closes.
   */
  const SERVICE_TABS = {
    [TAB_KIND.MONGODB]: {
      uriField: 'mongo_uri',
      prefix: 'mongo',
      disconnect: 'mongodb_disconnect',
    },
    [TAB_KIND.REDIS]: {
      uriField: 'redis_uri',
      prefix: 'redis',
      disconnect: 'redis_disconnect',
    },
  }

  /**
   * Open a datastore panel tab, or re-activate the one already open.
   *
   * Synchronous, unlike `connect`: there is no session to establish here. The
   * panel owns its own connection and reports the result, so the tab appears
   * immediately and shows its own spinner or error.
   */
  function openServiceTab(hostId, kind) {
    const spec = SERVICE_TABS[kind]
    const host = hosts.value.find(h => h.id === hostId)
    if (!spec || !host?.[spec.uriField]) return

    const existing = tabs.value.find(t => t.type === kind && t.hostId === hostId)
    if (existing) {
      activeTabId.value = existing.id
      return existing.id
    }

    const tab = {
      id: `${spec.prefix}-${hostId}-${Date.now()}`,
      type: kind,
      hostId,
      name: host.name,
      connected: true,
      connecting: false,
    }
    tabs.value = [...tabs.value, tab]
    activeTabId.value = tab.id
    return tab.id
  }

  function closeServiceTab(sessionId) {
    const tab = tabs.value.find(t => t.id === sessionId)
    tabs.value = tabs.value.filter(t => t.id !== sessionId)
    // Release the backend's pooled connections (and, for Redis, its SSH
    // tunnel) for this host, mirroring the per-host cleanup SSH tabs do on
    // disconnect.
    const spec = SERVICE_TABS[tab?.type]
    if (spec && !tabs.value.some(t => t.hostId === tab.hostId)) {
      invoke(spec.disconnect, { hostId: tab.hostId }).catch(() => {})
    }
    if (activeTabId.value === sessionId) {
      activeTabId.value = tabs.value.length > 0 ? tabs.value[0].id : null
    }
  }

  const openMongoTab = hostId => openServiceTab(hostId, TAB_KIND.MONGODB)
  const openRedisTab = hostId => openServiceTab(hostId, TAB_KIND.REDIS)

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

  async function updatePortForward(id, forward) {
    await invoke('update_port_forward', { id, forward })
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
    const [font_size, download_path, theme] = await Promise.all([
      invoke('get_setting', { key: 'font_size' }),
      invoke('get_setting', { key: 'download_path' }),
      invoke('get_setting', { key: 'theme' }),
    ])
    settings.value = {
      font_size: font_size || '14',
      download_path: download_path || '',
      // SQLite is the record. localStorage is only a paint-time cache to stop
      // the wrong theme flashing before this resolves, so it must not win
      // here — a stale mirror would otherwise override the default.
      theme: theme || 'dark',
    }
    applyTheme(settings.value.theme)
    return settings.value
  }

  async function saveSettings(newSettings) {
    await Promise.all([
      invoke('set_setting', { key: 'font_size', value: String(newSettings.font_size || 14) }),
      invoke('set_setting', { key: 'download_path', value: newSettings.download_path || '' }),
      invoke('set_setting', { key: 'theme', value: newSettings.theme || 'dark' }),
    ])
    settings.value = { ...settings.value, ...newSettings }
    applyTheme(settings.value.theme)
  }

  return {
    hosts,
    tabs,
    activeTabId,
    activeTab,
    connectingHostId,
    loadHosts,
    bootstrap,
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
    openMongoTab,
    openRedisTab,
    openServiceTab,
    closeServiceTab,
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
    updatePortForward,
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
    getSecurityReport,
    runSecurityAudit,
    registerTerminal,
    unregisterTerminal,
  }
})