import { ref } from 'vue'
import { open, save } from '@tauri-apps/plugin-dialog'
import { invoke } from '../utils/invoke.js'
import { toast } from '../utils/toast.js'
import { buildEntries } from '../utils/mongoSelection.js'

const EMPTY_PROGRESS = {
  db: '',
  collection: '',
  stage: '',
  synced: 0,
  total: 0,
  percent: 0,
  detail: '',
}

/**
 * Backing up a MongoDB connection to a file, and restoring one back.
 *
 * Kept out of the panel because it is the bulk of the work: picking paths,
 * looping databases, tracking one cancellable operation at a time, and turning
 * tool failures into readable messages.
 *
 * `hostId` and `selection` are refs the caller owns; `onRestored` is called once
 * a restore finishes so the caller can refresh what it is showing.
 */
export function useMongoBackup(hostId, selection, onRestored) {
  const busy = ref(false)
  const currentAction = ref('')
  const progress = ref({ ...EMPTY_PROGRESS })
  const currentOpId = ref('')
  const aborting = ref(false)

  const restoreConfirm = ref({
    show: false,
    inputPath: '',
    isArchive: false,
    entries: [],
    sourceDbs: [],
    // Dropping the target is destructive and has no undo, so it is opt-in.
    dropFirst: false,
  })

  function resetOperationState() {
    busy.value = false
    currentAction.value = ''
    currentOpId.value = ''
    aborting.value = false
    progress.value = { ...EMPTY_PROGRESS }
  }

  function beginOperation(action) {
    busy.value = true
    currentAction.value = action
    currentOpId.value = crypto.randomUUID()
    aborting.value = false
    progress.value = { ...EMPTY_PROGRESS }
  }

  /**
   * A fresh op id for the next backend call. `with_mongo_op` registers and
   * unregisters per invocation, so a loop that reused one id left
   * `mongodb_cancel` with nothing to find in the gaps between calls.
   */
  function nextOpId() {
    currentOpId.value = crypto.randomUUID()
    return currentOpId.value
  }

  /**
   * Toast an operation error. Cancellations reset state and return true so
   * loops can stop; other errors are reported with `failedPrefix: err`.
   */
  function handleOperationError(err, cancelledMessage, failedPrefix) {
    if (String(err).includes('cancelled')) {
      resetOperationState()
      toast(cancelledMessage, 'info')
      return true
    }
    toast(`${failedPrefix}: ${err}`, 'error')
    return false
  }

  const selectedCount = () =>
    Array.from(selection.value.values()).reduce((n, set) => n + set.size, 0)

  // ---- Backup ----------------------------------------------------------

  async function backupFolder() {
    if (!hostId.value || selectedCount() === 0) return

    const outputDir = await open({
      directory: true,
      multiple: false,
      title: 'Select backup folder',
    })
    if (!outputDir) return

    await runBackup(outputDir, false)
  }

  async function backupArchive() {
    if (!hostId.value || selectedCount() === 0) return

    const selectedDbs = Array.from(selection.value.keys())
    // mongodump writes one archive per invocation and --db is singular, so
    // several databases cannot share one archive. Say so instead of silently
    // overwriting the file once per database.
    if (selectedDbs.length > 1) {
      toast(
        'An archive holds a single database. Select collections from one database, ' +
          'or back up to a folder to write all of them into one tree.',
        'error',
      )
      return
    }

    const outputFile = await save({
      title: 'Save backup archive',
      defaultPath: selectedDbs.length === 1 ? `${selectedDbs[0]}.gz` : 'mongodb_backup.gz',
      filters: [
        { name: 'Gzip archive', extensions: ['gz'] },
        { name: 'BSON archive', extensions: ['archive', 'bson'] },
        { name: 'All files', extensions: ['*'] },
      ],
    })
    if (!outputFile) return

    await runBackup(outputFile, true)
  }

  async function runBackup(outputPath, isArchive) {
    const entries = buildEntries(selection.value)
    // An archive is a single file: looping would overwrite it once per database
    // and report success for every one. backupArchive refuses that case, so
    // reaching here with more than one database is a bug.
    if (isArchive && entries.length > 1) {
      toast('An archive can only hold one database. Back up to a folder instead.', 'error')
      return
    }

    beginOperation(isArchive ? 'backup-archive' : 'backup-folder')

    for (const entry of entries) {
      if (aborting.value) break
      nextOpId()
      try {
        await invoke('mongodb_dump', {
          hostId: hostId.value,
          db: entry.db,
          collections: entry.collections,
          outputDir: outputPath,
          isArchive,
          opId: currentOpId.value,
        })
        toast(`Backed up ${entry.db}: ${entry.collections.join(', ')}`, 'success')
      } catch (err) {
        if (handleOperationError(err, `Cancelled ${entry.db}`, `Backup failed for ${entry.db}`)) {
          break
        }
      }
    }

    resetOperationState()
  }

  // ---- Restore ---------------------------------------------------------

  function openRestoreConfirm(inputPath, isArchive, sourceDbs = []) {
    restoreConfirm.value = {
      show: true,
      inputPath,
      isArchive,
      entries: buildEntries(selection.value),
      sourceDbs,
      dropFirst: false,
    }
  }

  function cancelRestore() {
    restoreConfirm.value.show = false
  }

  async function confirmRestore() {
    const { inputPath, isArchive, entries, sourceDbs, dropFirst } = restoreConfirm.value
    restoreConfirm.value.show = false
    await runRestore(inputPath, isArchive, entries, sourceDbs, dropFirst)
  }

  async function restoreFolder() {
    if (!hostId.value) return

    const inputDir = await open({
      directory: true,
      multiple: false,
      title: 'Select backup folder (mongodump output)',
    })
    if (!inputDir) return

    try {
      const sourceDbs = await invoke('scan_restore_folder', { path: inputDir })
      openRestoreConfirm(inputDir, false, sourceDbs)
    } catch (err) {
      toast(`Selected folder is not a valid backup: ${err}`, 'error')
    }
  }

  async function restoreFile() {
    if (!hostId.value) return

    const inputFile = await open({
      directory: false,
      multiple: false,
      title: 'Select backup archive',
      filters: [
        { name: 'MongoDB archives', extensions: ['gz', 'archive', 'bson'] },
        { name: 'All files', extensions: ['*'] },
      ],
    })
    if (!inputFile) return

    openRestoreConfirm(inputFile, true, [])
  }

  /**
   * The mongorestore invocations for a folder restore, each with its own
   * success, cancel, and failure wording.
   */
  function folderRestoreJobs(entries, sourceDbs) {
    if (entries.length === 0 && sourceDbs.length === 1) {
      // A folder representing a single database: tell mongorestore which one,
      // so it does not skip the files.
      const name = sourceDbs[0].name
      return [{
        db: name,
        collections: [],
        success: `Restored ${name}`,
        cancelled: 'Cancelled folder restore',
        failed: 'Folder restore failed',
      }]
    }
    if (entries.length === 0) {
      // Nothing selected and several databases: restore everything under it.
      return [{
        db: '',
        collections: [],
        success: 'Restored folder',
        cancelled: 'Cancelled folder restore',
        failed: 'Folder restore failed',
      }]
    }
    return entries.map(entry => ({
      db: entry.db,
      collections: entry.collections,
      success: `Restored ${entry.db}: ${entry.collections.join(', ')}`,
      cancelled: `Cancelled ${entry.db}`,
      failed: `Restore failed for ${entry.db}`,
    }))
  }

  async function runRestore(inputPath, isArchive, entries, sourceDbs = [], dropFirst = false) {
    if (!hostId.value) return

    beginOperation(isArchive ? 'restore-archive' : 'restore-folder')
    const hasSelection = entries.length > 0

    if (isArchive) {
      // An archive can hold many databases; one mongorestore takes them all.
      const includes = []
      for (const entry of entries) {
        for (const coll of entry.collections) includes.push(`${entry.db}.${coll}`)
      }

      try {
        await invoke('mongodb_restore_archive', {
          hostId: hostId.value,
          includes,
          inputPath,
          dropFirst,
          opId: currentOpId.value,
        })
        toast(
          hasSelection ? 'Restored selected collections from archive' : 'Restored archive',
          'success',
        )
      } catch (err) {
        handleOperationError(err, 'Cancelled archive restore', 'Archive restore failed')
      }
    } else {
      for (const job of folderRestoreJobs(entries, sourceDbs)) {
        if (aborting.value) break
        nextOpId()
        try {
          await invoke('mongodb_restore', {
            hostId: hostId.value,
            db: job.db,
            collections: job.collections,
            inputDir: inputPath,
            isArchive: false,
            dropFirst,
            opId: currentOpId.value,
          })
          toast(job.success, 'success')
        } catch (err) {
          if (handleOperationError(err, job.cancelled, job.failed)) break
        }
      }
    }

    resetOperationState()
    // Show what was just restored without needing the tab reopened.
    await onRestored?.()
  }

  async function cancel() {
    if (!busy.value || !currentOpId.value || aborting.value) return
    aborting.value = true
    try {
      await invoke('mongodb_cancel', { opId: currentOpId.value })
    } catch (err) {
      toast(`Failed to cancel: ${err}`, 'error')
      aborting.value = false
    }
  }

  /** Apply a progress event from the backend, ignoring other operations'. */
  function applyProgress(payload) {
    if (currentOpId.value && payload.opId && payload.opId !== currentOpId.value) return
    progress.value = {
      db: payload.db || '',
      collection: payload.collection || '',
      stage: payload.stage || '',
      synced: payload.synced || 0,
      total: payload.total || 0,
      detail: payload.detail || '',
      percent:
        payload.percent !== undefined
          ? payload.percent
          : payload.total > 0
            ? Math.round((payload.synced / payload.total) * 100)
            : 0,
    }
  }

  /** A cancellation event for the running operation. */
  function applyCancelled(payload) {
    if (currentOpId.value && payload.opId && payload.opId !== currentOpId.value) return
    resetOperationState()
  }

  return {
    busy,
    currentAction,
    progress,
    restoreConfirm,
    backupFolder,
    backupArchive,
    restoreFolder,
    restoreFile,
    confirmRestore,
    cancelRestore,
    cancel,
    applyProgress,
    applyCancelled,
  }
}
