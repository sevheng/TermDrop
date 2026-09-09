import { ref, onUnmounted, getCurrentInstance } from 'vue'

const LINGER_MS = 3000

/**
 * The transfer-progress list shown under the SFTP file browser. Rows are
 * keyed by remote path (or `folder:<path>` for archive downloads), updated
 * from `sftp-progress` events, and removed 3s after they finish.
 */
export function useSftpTransfers() {
  const transfers = ref([])

  const timers = new Set()
  function removeLater(file) {
    const timer = setTimeout(() => {
      timers.delete(timer)
      transfers.value = transfers.value.filter(t => t.file !== file)
    }, LINGER_MS)
    timers.add(timer)
  }

  /** Apply one `sftp-progress` payload: {file, bytes_transferred, total_bytes}. */
  function handleProgress(p) {
    const now = Date.now()
    const fileName = p.file.split('/').pop() || p.file
    const existing = transfers.value.find(t => t.file === p.file)
    if (existing) {
      const dt = (now - existing.lastUpdate) / 1000
      if (dt > 0) {
        const db = p.bytes_transferred - existing.bytes
        existing.speed = db / dt
      }
      existing.bytes = p.bytes_transferred
      existing.total = p.total_bytes
      existing.lastUpdate = now
      if (existing.bytes >= existing.total && !existing.done) {
        existing.done = true
        removeLater(p.file)
      }
    } else {
      const isDone = p.total_bytes === 0 || p.bytes_transferred >= p.total_bytes
      transfers.value.push({
        file: p.file,
        fileName,
        bytes: p.bytes_transferred,
        total: p.total_bytes,
        speed: 0,
        lastUpdate: now,
        done: isDone,
      })
      if (isDone) {
        removeLater(p.file)
      }
    }
  }

  /** Add an indeterminate row for a folder archive download; returns its key. */
  function beginFolderTransfer(file) {
    const key = `folder:${file.path}`
    transfers.value.push({
      file: key,
      fileName: `📁 ${file.name}`,
      bytes: 0,
      total: 0,
      speed: 0,
      lastUpdate: Date.now(),
      done: false,
    })
    return key
  }

  /** Mark a folder row done with a suffix like "(saved)" or "(failed)". */
  function finishFolderTransfer(key, file, suffix) {
    const t = transfers.value.find(x => x.file === key)
    if (t) {
      t.done = true
      t.fileName = `📁 ${file.name} ${suffix}`
      removeLater(key)
    }
  }

  function clearTimers() {
    timers.forEach(clearTimeout)
    timers.clear()
  }

  if (getCurrentInstance()) onUnmounted(clearTimers)

  return { transfers, handleProgress, beginFolderTransfer, finishFolderTransfer, clearTimers }
}
