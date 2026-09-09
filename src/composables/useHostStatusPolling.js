import { ref, shallowRef } from 'vue'
import { invoke } from '../utils/invoke.js'
import { formatBytes, formatRate } from '../utils/format.js'

export const EMPTY_STATUS = {
  load: '',
  ram: '',
  disk: '',
  uptime: '',
  os: '',
  cores: '',
  netDown: '',
  netUp: '',
}

const STATUS_POLL_MS = 5000
const PANEL_POLL_MS = 3000

/**
 * Sum rx/tx byte counters from the `netdev` field of get_system_stats
 * ("iface: rx tx" per line), ignoring the loopback interface.
 */
export function computeNetTotals(netdev) {
  let rx = 0
  let tx = 0
  for (const line of netdev.split('\n')) {
    const parts = line.trim().split(/\s+/)
    if (parts.length >= 3) {
      const iface = parts[0].replace(':', '')
      if (iface === 'lo') continue
      rx += parseInt(parts[1]) || 0
      tx += parseInt(parts[2]) || 0
    }
  }
  return { rx, tx }
}

/**
 * Status-bar polling (get_system_stats every 5s) and the expandable
 * system panel (get_system_panel every 3s) for one host. The store keeps
 * the last status and network counters per host so a remounted tab warms
 * up instantly and rates survive tab switches.
 *
 * `hostId` is a getter; `isDisconnected` a ref. Callers decide when to
 * start and stop (active tab, visibility, connection state).
 */
export function useHostStatusPolling({ hostId, isDisconnected, store }) {
  const status = ref({ ...EMPTY_STATUS })
  const statusLoading = ref(false)
  const statusError = ref('')
  let statusInterval = null

  const processes = shallowRef([]) // replaced wholesale every poll
  const network = ref(null)
  const diskInfo = ref(null)
  const sysLoading = ref(false)
  let sysPollInterval = null

  async function fetchSystemStatus() {
    const id = hostId()
    if (!id || isDisconnected.value) return
    statusLoading.value = true
    try {
      const result = await invoke('get_system_stats', { hostId: id })

      // Compute network rates
      let netDown = ''
      let netUp = ''
      if (result.netdev) {
        const { rx: rxTotal, tx: txTotal } = computeNetTotals(result.netdev)
        const now = Date.now()
        const prev = store.getNetStats(id)
        if (prev.time > 0 && prev.rx > 0 && prev.tx > 0) {
          const elapsed = (now - prev.time) / 1000
          if (elapsed > 0) {
            const rxRate = (rxTotal - prev.rx) / elapsed
            const txRate = (txTotal - prev.tx) / elapsed
            netDown = formatRate(rxRate)
            netUp = formatRate(txRate)
          }
        } else {
          // First fetch: show cumulative totals instead of dash
          netDown = formatBytes(rxTotal)
          netUp = formatBytes(txTotal)
        }
        store.setNetStats(id, { rx: rxTotal, tx: txTotal, time: now })
      }

      const osParts = [result.os, result.kernel, result.arch].filter(Boolean)
      const data = {
        load: result.load || '',
        ram: result.ram || '',
        disk: result.disk || '',
        uptime: result.uptime || '',
        os: osParts.join(' · '),
        cores: result.cores || '',
        netDown,
        netUp,
      }
      status.value = data
      store.setSystemStatus(id, data)
      statusError.value = ''
    } catch (err) {
      console.warn('get_system_stats failed:', err)
      statusError.value = String(err).replace(/^Error: /, '')
    } finally {
      statusLoading.value = false
    }
  }

  function startStatusPolling() {
    if (statusInterval) clearInterval(statusInterval)
    const id = hostId()
    if (!id) return
    // Don't poll when page is hidden
    if (document.hidden) return
    // Read from cache immediately
    const cached = store.getSystemStatus(id)
    if (cached) {
      status.value = cached
    }
    // Fetch immediately, then every 5s
    fetchSystemStatus()
    statusInterval = setInterval(fetchSystemStatus, STATUS_POLL_MS)
  }

  function stopStatusPolling() {
    if (statusInterval) {
      clearInterval(statusInterval)
      statusInterval = null
    }
  }

  /** Clear the status bar, e.g. after a disconnect. */
  function resetStatus() {
    status.value = { ...EMPTY_STATUS }
    statusError.value = ''
  }

  async function fetchPanelData(includeDisk = false) {
    const id = hostId()
    if (!id) return
    try {
      const panel = await invoke('get_system_panel', { hostId: id })
      processes.value = panel.processes || []
      network.value = panel.network || null
      if (includeDisk) {
        diskInfo.value = panel.disk || null
      }
    } catch (err) {
      console.error('get_system_panel failed:', err)
    }
  }

  async function loadSystemData() {
    if (!hostId()) return
    sysLoading.value = true
    await fetchPanelData(true)
    sysLoading.value = false
  }

  function startSysPolling() {
    if (sysPollInterval) clearInterval(sysPollInterval)
    if (!hostId()) return
    loadSystemData()
    sysPollInterval = setInterval(() => {
      fetchPanelData(false)
    }, PANEL_POLL_MS)
  }

  function stopSysPolling() {
    if (sysPollInterval) {
      clearInterval(sysPollInterval)
      sysPollInterval = null
    }
  }

  return {
    status,
    statusLoading,
    statusError,
    startStatusPolling,
    stopStatusPolling,
    resetStatus,
    processes,
    network,
    diskInfo,
    sysLoading,
    startSysPolling,
    stopSysPolling,
  }
}
