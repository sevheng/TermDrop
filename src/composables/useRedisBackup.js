import { ref } from 'vue'
import { save, open } from '@tauri-apps/plugin-dialog'
import { invoke } from '../utils/invoke.js'
import { toast } from '../utils/toast.js'
import { DEFAULT_MAX_DUMP_KEY_BYTES } from '../utils/redisLimits.js'
import { defaultBackupName, summarizeOp } from '../utils/redisBackup.js'

/**
 * Backing up and restoring one Redis database.
 *
 * Mirrors `useMongoBackup`, including the rule that matters most: a **fresh**
 * op id per invocation. A single reused id meant a cancel could land on a
 * later operation, and progress from one run could drive another's bar.
 */
export function useRedisBackup(hostId, db, connectionName, onFinished) {
  const busy = ref(false)
  const currentAction = ref('')
  const progress = ref(null)
  const restoreConfirm = ref({ show: false, path: '', header: null, replace: false, flushFirst: false, targetDb: 0 })

  let opId = null
  function nextOpId() {
    opId = crypto.randomUUID()
    return opId
  }

  function applyProgress(payload) {
    // Filtered by op id: a stale event from a cancelled run must not move the
    // bar of the one that replaced it.
    if (!opId || payload?.opId !== opId) return
    progress.value = payload
  }

  function applyCancelled(payload) {
    if (!opId || payload?.opId !== opId) return
    progress.value = null
  }

  function finish(summary, verb) {
    const { message, type } = summarizeOp(summary, verb)
    toast(message, type)
  }

  async function backup(pattern) {
    const path = await save({
      defaultPath: defaultBackupName(connectionName.value, db.value, pattern),
      filters: [{ name: 'TermDrop Redis backup', extensions: ['tdredis'] }],
    })
    if (!path) return

    busy.value = true
    currentAction.value = 'Backing up'
    progress.value = null
    try {
      const summary = await invoke('redis_export', {
        opId: nextOpId(),
        hostId: hostId.value,
        db: db.value,
        pattern: pattern || null,
        outputPath: path,
        maxKeyBytes: DEFAULT_MAX_DUMP_KEY_BYTES,
      })
      finish(summary, 'Backed up')
    } catch (err) {
      // The backend returns this exact string when the cancel flag was seen.
      if (String(err).includes('cancelled')) toast('Backup cancelled', 'info')
      else toast(`Backup failed: ${err}`, 'error')
    } finally {
      busy.value = false
      currentAction.value = ''
      progress.value = null
    }
  }

  /** Read the file's header so the dialog can describe what is about to happen. */
  async function chooseRestoreFile() {
    const path = await open({
      multiple: false,
      filters: [{ name: 'TermDrop Redis backup', extensions: ['tdredis'] }],
    })
    if (!path) return

    try {
      const header = await invoke('redis_backup_info', { path })
      restoreConfirm.value = {
        show: true,
        path,
        header,
        replace: false,
        flushFirst: false,
        targetDb: header.dbs?.[0] ?? db.value,
      }
    } catch (err) {
      toast(`That file cannot be restored: ${err}`, 'error')
    }
  }

  async function confirmRestore() {
    const { path, targetDb, replace, flushFirst } = restoreConfirm.value
    restoreConfirm.value = { ...restoreConfirm.value, show: false }

    busy.value = true
    currentAction.value = 'Restoring'
    progress.value = null
    try {
      const summary = await invoke('redis_import', {
        opId: nextOpId(),
        hostId: hostId.value,
        db: targetDb,
        inputPath: path,
        replace,
        flushFirst,
      })
      finish(summary, 'Restored')
      await onFinished?.()
    } catch (err) {
      if (String(err).includes('cancelled')) toast('Restore cancelled', 'info')
      else toast(`Restore failed: ${err}`, 'error')
    } finally {
      busy.value = false
      currentAction.value = ''
      progress.value = null
    }
  }

  function cancelRestore() {
    restoreConfirm.value = { ...restoreConfirm.value, show: false }
  }

  function cancel() {
    if (opId) invoke('redis_cancel', { opId }).catch(() => {})
  }

  return {
    busy,
    currentAction,
    progress,
    restoreConfirm,
    backup,
    chooseRestoreFile,
    confirmRestore,
    cancelRestore,
    cancel,
    applyProgress,
    applyCancelled,
  }
}
