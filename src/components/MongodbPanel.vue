<template>
  <div class="h-full flex flex-col bg-[#1e1e1e]">
    <!-- Header with host info -->
    <div class="flex items-center justify-between px-3 py-2 border-b border-[#3c3c3c] bg-[#252526]">
      <div class="flex items-center gap-2 min-w-0">
        <Database :size="16" class="text-[#007acc] shrink-0" />
        <span class="text-sm font-medium text-[#cccccc] truncate">{{ host?.name || 'MongoDB' }}</span>
        <span
          class="text-[10px] px-1.5 py-0.5 rounded shrink-0"
          :class="hasLocalUri ? 'bg-[#0e639c]/30 text-[#75beff]' : 'bg-[#6e6e6e]/30 text-[#858585]'"
        >
          {{ hasLocalUri ? 'Sync mode' : 'Dump/Restore' }}
        </span>
      </div>
      <div class="flex items-center gap-2 text-[10px] min-w-0">
        <template v-if="!hasLocalUri">
          <span
            class="px-1.5 py-0.5 rounded bg-[#0e639c]/20 text-[#75beff] truncate max-w-[16rem]"
            :title="remoteUri"
          >
            Remote
          </span>
        </template>
        <template v-else>
          <div
            class="flex items-center gap-1 px-2 py-1 rounded bg-[#3c3c3c] text-[#cccccc]"
            :title="`Remote: ${remoteUri}\nLocal: ${localUri}`"
          >
            <span :class="isRemoteToLocal ? 'text-[#75beff]' : 'text-[#89d185]'">
              {{ isRemoteToLocal ? 'Remote' : 'Local' }}
            </span>
            <span class="text-[#858585]">→</span>
            <span :class="isRemoteToLocal ? 'text-[#89d185]' : 'text-[#75beff]'">
              {{ isRemoteToLocal ? 'Local' : 'Remote' }}
            </span>
          </div>
        </template>
      </div>
    </div>

    <!-- DB Trees row -->
    <div class="flex-1 flex overflow-hidden">
      <!-- Source (From) panel -->
      <div
        class="flex flex-col min-w-0"
        :class="[
          hasLocalUri ? 'flex-1 border-r border-[#3c3c3c]' : 'w-full',
          isRemoteToLocal ? '' : 'opacity-80'
        ]"
      >
        <div class="px-2 py-1.5 border-b border-[#3c3c3c] flex items-center justify-between bg-[#252526]/50">
          <div class="flex items-center gap-1.5">
            <span
              class="text-[10px] font-semibold uppercase"
              :class="isRemoteToLocal ? 'text-[#75beff]' : 'text-[#89d185]'"
            >
              {{ isRemoteToLocal ? 'Remote' : 'Local' }}
            </span>
            <span
              v-if="hasLocalUri"
              class="text-[9px] px-1 py-0.5 rounded border"
              :class="isRemoteToLocal
                ? 'border-[#75beff]/30 text-[#75beff] bg-[#0e639c]/10'
                : 'border-[#89d185]/30 text-[#89d185] bg-[#388a34]/10'"
            >
              From
            </span>
          </div>
          <button
            @click="isRemoteToLocal ? loadRemoteDatabases() : loadLocalDatabases()"
            class="text-[#858585] hover:text-[#cccccc] p-0.5"
            :title="isRemoteToLocal ? 'Refresh remote' : 'Refresh local'"
          >
            <RefreshCw :size="10" />
          </button>
        </div>
        <DbTree
          :databases="sourceDatabases"
          :expanded-dbs="sourceExpandedDbs"
          :selected-collections="selectedCollections"
          :selectable="true"
          :loading="isRemoteToLocal ? loadingRemote : loadingLocal"
          @toggle-db="toggleSourceDb"
          @toggle-db-selection="toggleDbSelection"
          @toggle-collection="toggleCollection"
        />
      </div>

      <!-- Direction swap + destination (sync mode only) -->
      <template v-if="hasLocalUri">
        <div class="w-12 flex flex-col items-center justify-center border-r border-[#3c3c3c] bg-[#1e1e1e]">
          <button
            @click="flipDirection"
            class="p-2 rounded-lg hover:bg-[#3c3c3c] text-[#858585] hover:text-[#cccccc] transition-colors"
            title="Swap direction"
          >
            <ArrowRightLeft :size="16" />
          </button>
          <span class="text-[9px] font-medium text-[#6e6e6e] mt-1.5">
            {{ isRemoteToLocal ? 'R→L' : 'L→R' }}
          </span>
        </div>

        <!-- Destination (To) panel -->
        <div class="flex-1 flex flex-col min-w-0"
          :class="isRemoteToLocal ? 'opacity-80' : ''"
        >
          <div class="px-2 py-1.5 border-b border-[#3c3c3c] flex items-center justify-between bg-[#252526]/50">
            <div class="flex items-center gap-1.5">
              <span
                class="text-[10px] font-semibold uppercase"
                :class="isRemoteToLocal ? 'text-[#89d185]' : 'text-[#75beff]'"
              >
                {{ isRemoteToLocal ? 'Local' : 'Remote' }}
              </span>
              <span
                class="text-[9px] px-1 py-0.5 rounded border"
                :class="isRemoteToLocal
                  ? 'border-[#89d185]/30 text-[#89d185] bg-[#388a34]/10'
                  : 'border-[#75beff]/30 text-[#75beff] bg-[#0e639c]/10'"
              >
                To
              </span>
            </div>
            <button
              @click="isRemoteToLocal ? loadLocalDatabases() : loadRemoteDatabases()"
              class="text-[#858585] hover:text-[#cccccc] p-0.5"
              :title="isRemoteToLocal ? 'Refresh local' : 'Refresh remote'"
            >
              <RefreshCw :size="10" />
            </button>
          </div>
          <DbTree
            :databases="destDatabases"
            :expanded-dbs="destExpandedDbs"
            :selected-collections="new Map()"
            :selectable="false"
            :loading="isRemoteToLocal ? loadingLocal : loadingRemote"
            @toggle-db="toggleDestDb"
          />
        </div>
      </template>
    </div>

    <!-- Action footer -->
    <div class="border-t border-[#3c3c3c] px-4 py-3.5 space-y-3 shrink-0 bg-[#252526]">
      <!-- Drop checkbox only for sync mode -->
      <label v-if="hasLocalUri" class="flex items-center gap-1.5 text-[11px] text-[#cccccc] cursor-pointer">
        <input type="checkbox" v-model="dropFirst" class="accent-[#007acc]" />
        Drop existing collections before sync
      </label>

      <div v-if="syncing" class="space-y-1">
        <div class="flex items-center justify-between text-[10px] text-[#6e6e6e]">
          <span>{{ syncProgress.stage }} {{ syncProgress.collection }}</span>
          <button
            @click="cancelOperation"
            class="text-[#f44336] hover:text-red-300 underline"
          >
            Cancel
          </button>
        </div>
        <div class="h-1.5 bg-[#3c3c3c] rounded-full overflow-hidden">
          <div
            class="h-full bg-[#007acc] rounded-full transition-all duration-300"
            :style="{ width: syncProgress.percent + '%' }"
          />
        </div>
        <div class="flex justify-between text-[10px] text-[#6e6e6e]">
          <span>{{ syncProgress.synced }} / {{ syncProgress.total }}</span>
        </div>
      </div>

      <!-- Selected summary -->
      <div v-if="selectedCount > 0" class="flex flex-wrap gap-1 mb-2">
        <span class="text-[10px] text-[#858585]">{{ selectedCount }} selected:</span>
        <span
          v-for="[db, set] in selectedCollections"
          :key="db"
          class="inline-flex items-center gap-1 text-[10px] bg-[#3c3c3c] text-[#cccccc] px-1.5 py-0.5 rounded"
        >
          {{ db }}({{ set.size }})
          <button @click="clearDbSelection(db)" class="hover:text-red-400">
            <X :size="8" />
          </button>
        </span>
        <button @click="clearSelection" class="text-[10px] text-[#858585] hover:text-[#cccccc] underline">
          Clear all
        </button>
      </div>

      <!-- Sync button (when local URI configured) -->
      <button
        v-if="hasLocalUri"
        @click="startSync"
        :disabled="!canSync || syncing"
        class="w-full py-2 text-xs font-medium rounded flex items-center justify-center gap-1.5 transition-colors"
        :class="canSync && !syncing
          ? 'bg-[#0e639c] hover:bg-[#1177bb] text-white'
          : 'bg-[#3c3c3c] text-[#6e6e6e] cursor-not-allowed'"
      >
        <Play v-if="!syncing" :size="12" />
        <Loader2 v-else :size="12" class="animate-spin" />
        {{ syncButtonLabel }}
      </button>

      <!-- Dump/Restore buttons (always available, operate on Remote) -->
      <div class="grid grid-cols-2 gap-2">
        <div class="flex flex-col gap-2">
          <button
            @click="startDumpFolder"
            :disabled="!canDumpRestore || syncing"
            class="flex-1 py-1 text-xs font-medium rounded flex items-center justify-center gap-1.5 transition-colors"
            :class="canDumpRestore && !syncing
              ? 'bg-[#0e639c] hover:bg-[#1177bb] text-white'
              : 'bg-[#3c3c3c] text-[#6e6e6e] cursor-not-allowed'"
          >
            <Download v-if="!syncing" :size="12" />
            <Loader2 v-else :size="12" class="animate-spin" />
            {{ syncing && currentAction === 'dump-folder' ? 'Dumping...' : `Dump folder (${dumpSourceLabel})` }}
          </button>
          <button
            @click="startDumpArchive"
            :disabled="!canDumpRestore || syncing"
            class="flex-1 py-1 text-xs font-medium rounded flex items-center justify-center gap-1.5 transition-colors"
            :class="canDumpRestore && !syncing
              ? 'bg-[#0e639c] hover:bg-[#1177bb] text-white'
              : 'bg-[#3c3c3c] text-[#6e6e6e] cursor-not-allowed'"
          >
            <Download v-if="!syncing" :size="12" />
            <Loader2 v-else :size="12" class="animate-spin" />
            {{ syncing && currentAction === 'dump-archive' ? 'Dumping...' : `Dump archive (${dumpSourceLabel})` }}
          </button>
        </div>
        <div class="flex flex-col gap-2">
          <button
            @click="startRestoreFolder"
            :disabled="!canRestore || syncing"
            class="flex-1 py-1 text-xs font-medium rounded flex items-center justify-center gap-1.5 transition-colors"
            :class="canRestore && !syncing
              ? 'bg-[#388a34] hover:bg-[#43a047] text-white'
              : 'bg-[#3c3c3c] text-[#6e6e6e] cursor-not-allowed'"
          >
            <Upload v-if="!syncing" :size="12" />
            <Loader2 v-else :size="12" class="animate-spin" />
            {{ syncing && currentAction === 'restore-folder' ? 'Restoring...' : `Restore folder → ${restoreTargetLabel}` }}
          </button>
          <button
            @click="startRestoreFile"
            :disabled="!canRestore || syncing"
            class="flex-1 py-1 text-xs font-medium rounded flex items-center justify-center gap-1.5 transition-colors"
            :class="canRestore && !syncing
              ? 'bg-[#388a34] hover:bg-[#43a047] text-white'
              : 'bg-[#3c3c3c] text-[#6e6e6e] cursor-not-allowed'"
          >
            <Upload v-if="!syncing" :size="12" />
            <Loader2 v-else :size="12" class="animate-spin" />
            {{ syncing && currentAction === 'restore-archive' ? 'Restoring...' : `Restore file → ${restoreTargetLabel}` }}
          </button>
        </div>
      </div>
    </div>

    <!-- Restore confirmation modal -->
    <ModalShell :show="restoreConfirm.show" dim="bg-black/60" z="z-[100]" panel-class="p-5 w-[28rem] shadow-xl">
        <h3 class="text-base font-semibold text-[#cccccc] mb-3">
          Confirm restore into <span class="text-[#75beff]">{{ restoreTargetLabel }}</span>
        </h3>
        <div class="space-y-2 text-sm text-[#cccccc]">
          <p>
            Source {{ restoreConfirm.isArchive ? 'archive' : 'folder' }}:
            <span class="font-mono text-[#89d185] break-all">{{ restoreConfirm.inputPath }}</span>
          </p>
          <div v-if="!restoreConfirm.isArchive && restoreConfirm.sourceDbs.length > 0">
            <p class="text-[#858585] mb-1">Data found in folder:</p>
            <ul class="max-h-32 overflow-y-auto bg-[#1e1e1e] rounded p-2 space-y-1 text-xs">
              <li v-for="db in restoreConfirm.sourceDbs" :key="db.name">
                <span class="text-[#75beff]">{{ db.name }}</span>:
                <span class="text-[#cccccc]">{{ db.collections.map(c => c.name).join(', ') }}</span>
              </li>
            </ul>
          </div>
          <template v-if="restoreConfirm.entries.length > 0">
            <p>
              Target filter database{{ restoreConfirm.entries.length > 1 ? 's' : '' }}:
              <span class="font-mono text-[#75beff]">
                {{ restoreConfirm.entries.map(e => e.db).join(', ') }}
              </span>
            </p>
            <div>
              <p class="text-[#858585] mb-1">Collections to restore:</p>
              <ul class="max-h-32 overflow-y-auto bg-[#1e1e1e] rounded p-2 space-y-0.5 text-xs">
                <li v-for="entry in restoreConfirm.entries" :key="entry.db">
                  <span class="text-[#75beff]">{{ entry.db }}</span>:
                  <span class="text-[#cccccc]">{{ entry.collections.join(', ') }}</span>
                </li>
              </ul>
            </div>
          </template>
          <template v-else-if="!restoreConfirm.isArchive && restoreConfirm.sourceDbs.length === 1">
            <p class="text-[#75beff]">
              Will restore database <span class="font-mono text-[#75beff]">{{ restoreConfirm.sourceDbs[0].name }}</span>
              (all collections found in the folder).
            </p>
          </template>
          <template v-else-if="!restoreConfirm.isArchive && restoreConfirm.sourceDbs.length > 1">
            <p class="text-[#75beff]">
              Will restore all databases found in the folder:
              <span class="font-mono text-[#75beff]">
                {{ restoreConfirm.sourceDbs.map(d => d.name).join(', ') }}
              </span>
            </p>
          </template>
          <p v-else class="text-[#75beff]">
            No database selected — everything in the source will be restored.
          </p>
          <label class="flex items-start gap-2 pt-1 cursor-pointer">
            <input
              type="checkbox"
              v-model="restoreConfirm.dropFirst"
              class="accent-[#f44336] mt-0.5 shrink-0"
            />
            <span class="text-xs">
              Drop existing collections in the target first
            </span>
          </label>
          <p v-if="restoreConfirm.dropFirst" class="text-[#f44336] text-xs">
            Existing collections in the target database will be dropped before
            restoring. This cannot be undone.
          </p>
          <p v-else class="text-[#858585] text-xs">
            Existing documents are kept. Documents whose <span class="font-mono">_id</span>
            already exists will be reported as failures, not overwritten.
          </p>
        </div>
        <div class="flex justify-end gap-2 mt-5">
          <button
            @click="cancelRestore"
            class="px-3 py-1.5 text-sm text-[#858585] hover:text-[#cccccc] rounded hover:bg-[#2a2d2e] transition-colors"
          >
            Cancel
          </button>
          <button
            @click="confirmRestore"
            class="px-3 py-1.5 text-sm text-white rounded transition-colors"
            :class="restoreConfirm.dropFirst
              ? 'bg-[#f44336] hover:bg-[#d32f2f]'
              : 'bg-[#007acc] hover:bg-[#1f8ad2]'"
          >
            Restore
          </button>
        </div>
    </ModalShell>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, watch } from 'vue'
import { invoke } from '../utils/invoke.js'
import { open, save } from '@tauri-apps/plugin-dialog'
import {
  Database,
  Play,
  Loader2,
  RefreshCw,
  X,
  Download,
  Upload,
  ArrowRightLeft,
} from 'lucide-vue-next'
import DbTree from './DbTree.vue'
import ModalShell from './ModalShell.vue'
import { toast } from '../utils/toast.js'
import { useListenerGroup } from '../composables/useListenerGroup.js'
import { useMongoSide, fetchCollections } from '../composables/useMongoSide.js'
import {
  dbSelectionState,
  countSelected,
  buildEntries,
  toggleCollection as toggleCollectionIn,
  withoutDb,
  withAllCollections,
} from '../utils/mongoSelection.js'

const props = defineProps({
  hostId: { type: Number, required: true },
})

const host = ref(null)
const remoteUri = computed(() => host.value?.mongo_uri || '')
const localUri = computed(() => host.value?.mongo_local_uri || '')
const hasLocalUri = computed(() => !!localUri.value)

// The two physical sides. "Source" and "dest" are roles that flip with the direction.
const remote = useMongoSide(remoteUri, 'remote')
const local = useMongoSide(localUri, 'local')
const loadingRemote = remote.loading
const loadingLocal = local.loading
const loadRemoteDatabases = remote.loadDatabases
const loadLocalDatabases = local.loadDatabases

const selectedCollections = ref(new Map())
const syncing = ref(false)
const currentAction = ref('') // 'sync' | 'dump-folder' | 'dump-archive' | 'restore-folder' | 'restore-archive'
const dropFirst = ref(false)
const isRemoteToLocal = ref(true) // true = Remote→Local, false = Local→Remote
const currentOpId = ref('')
const aborting = ref(false)

const EMPTY_PROGRESS = { db: '', collection: '', stage: '', synced: 0, total: 0, percent: 0 }
const syncProgress = ref({ ...EMPTY_PROGRESS })

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
  syncing.value = false
  currentAction.value = ''
  currentOpId.value = ''
  aborting.value = false
  syncProgress.value = { ...EMPTY_PROGRESS }
}

/** Mark an operation as running with a fresh op id for cancellation. */
function beginOperation(action) {
  syncing.value = true
  currentAction.value = action
  currentOpId.value = crypto.randomUUID()
  aborting.value = false
  syncProgress.value = { ...EMPTY_PROGRESS }
}

/**
 * A fresh op id for the next backend call. `with_mongo_op` registers and
 * unregisters per invocation, so a loop that reused one id left `mongodb_cancel`
 * with nothing to find in the gaps between calls.
 */
function nextOpId() {
  currentOpId.value = crypto.randomUUID()
  return currentOpId.value
}

/**
 * Toast an operation error. Cancellations reset the panel and return true
 * so loops can stop; other errors are reported with `failedPrefix: err`.
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

// Source/dest computed based on direction
const sourceSide = computed(() => isRemoteToLocal.value ? remote : local)
const destSide = computed(() => isRemoteToLocal.value ? local : remote)
const sourceUri = computed(() => isRemoteToLocal.value ? remoteUri.value : localUri.value)
const destUri = computed(() => isRemoteToLocal.value ? localUri.value : remoteUri.value)
const sourceDatabases = computed(() => sourceSide.value.databases.value)
const destDatabases = computed(() => destSide.value.databases.value)
const sourceExpandedDbs = computed(() => sourceSide.value.expandedDbs.value)
const destExpandedDbs = computed(() => destSide.value.expandedDbs.value)

const selectedCount = computed(() => countSelected(selectedCollections.value))

const canSync = computed(() => {
  return sourceUri.value && destUri.value && selectedCount.value > 0 && !syncing.value
})

const canDumpRestore = computed(() => {
  // Dump reads from whichever side the selection tree is showing.
  return sourceUri.value && selectedCount.value > 0 && !syncing.value
})

const canRestore = computed(() => {
  // Restore writes into the destination side; the folder/archive decides what.
  return destUri.value && !syncing.value
})

/** Names the side dump reads from / restore writes to, for button labels. */
const dumpSourceLabel = computed(() => (isRemoteToLocal.value ? 'Remote' : 'Local'))
const restoreTargetLabel = computed(() => (isRemoteToLocal.value ? 'Local' : 'Remote'))

const syncButtonLabel = computed(() => {
  if (syncing.value) return 'Syncing...'
  const dir = isRemoteToLocal.value ? 'Remote → Local' : 'Local → Remote'
  return `Sync ${selectedCount.value} collection${selectedCount.value === 1 ? '' : 's'} (${dir})`
})

async function toggleDbSelection(db) {
  const state = dbSelectionState(selectedCollections.value, db)

  if (state === 'all') {
    selectedCollections.value = withoutDb(selectedCollections.value, db.name)
    return
  }
  const side = sourceSide.value
  if (!side.expandedDbs.value.has(db.name)) {
    const newExpanded = new Set(side.expandedDbs.value)
    newExpanded.add(db.name)
    side.expandedDbs.value = newExpanded
  }
  if (db.collections.length === 0 && !db.loading) {
    await fetchCollections(sourceUri.value, db, 'source')
  }
  selectedCollections.value = withAllCollections(selectedCollections.value, db)
}

function toggleCollection(db, coll) {
  selectedCollections.value = toggleCollectionIn(selectedCollections.value, db, coll)
}

function clearDbSelection(db) {
  selectedCollections.value = withoutDb(selectedCollections.value, db)
}

function clearSelection() {
  selectedCollections.value = new Map()
}

/** Expand or collapse a database on one side, loading its collections on first expand. */
function toggleExpanded(side, dbName, uriValue, roleLabel) {
  const newSet = new Set(side.expandedDbs.value)
  if (newSet.has(dbName)) {
    newSet.delete(dbName)
  } else {
    newSet.add(dbName)
    const db = side.databases.value.find(d => d.name === dbName)
    if (db && db.collections.length === 0 && !db.loading) {
      fetchCollections(uriValue, db, roleLabel)
    }
  }
  side.expandedDbs.value = newSet
}

function toggleSourceDb(dbName) {
  toggleExpanded(sourceSide.value, dbName, sourceUri.value, 'source')
}

function toggleDestDb(dbName) {
  toggleExpanded(destSide.value, dbName, destUri.value, 'dest')
}

function flipDirection() {
  isRemoteToLocal.value = !isRemoteToLocal.value
  clearSelection()
}

async function loadHost() {
  try {
    host.value = await invoke('get_host_by_id', { id: props.hostId })
  } catch (err) {
    toast('Failed to load host: ' + err, 'error')
  }
}

async function loadConfiguredDatabases() {
  if (remoteUri.value) {
    await remote.loadDatabases()
  }
  if (localUri.value) {
    await local.loadDatabases()
  }
}

async function startSync() {
  if (!sourceUri.value || !destUri.value || selectedCount.value === 0) return

  const entries = buildEntries(selectedCollections.value)
  beginOperation('sync')

  for (const entry of entries) {
    if (aborting.value) break
    nextOpId()
    try {
      await invoke('mongodb_sync', {
        remoteUri: sourceUri.value,
        localUri: destUri.value,
        db: entry.db,
        collections: entry.collections,
        dropFirst: dropFirst.value,
        opId: currentOpId.value,
      })
      toast(`Synced ${entry.db}: ${entry.collections.join(', ')}`, 'success')
    } catch (err) {
      if (handleOperationError(err, `Cancelled ${entry.db}`, `Sync failed for ${entry.db}`)) break
    }
  }

  resetOperationState()
  // Refresh the destination side
  await destSide.value.loadDatabases()
}

async function startDumpFolder() {
  if (!sourceUri.value || selectedCount.value === 0) return

  const outputDir = await open({
    directory: true,
    multiple: false,
    title: 'Select dump output folder',
  })
  if (!outputDir) return

  await runDump(outputDir, false)
}

async function startDumpArchive() {
  if (!sourceUri.value || selectedCount.value === 0) return

  const selectedDbs = Array.from(selectedCollections.value.keys())
  // mongodump writes one archive per invocation and --db is singular, so there
  // is no way to put several databases in one archive. Say so instead of
  // silently overwriting the file once per database.
  if (selectedDbs.length > 1) {
    toast(
      'An archive holds a single database. Select collections from one database, ' +
        'or use Dump folder to write all selected databases into one tree.',
      'error',
    )
    return
  }
  const defaultName =
    selectedDbs.length === 1 ? `${selectedDbs[0]}.gz` : 'mongodb_dump.gz'

  const outputFile = await save({
    title: 'Save dump archive',
    defaultPath: defaultName,
    filters: [
      { name: 'Gzip archive', extensions: ['gz'] },
      { name: 'BSON archive', extensions: ['archive', 'bson'] },
      { name: 'All files', extensions: ['*'] },
    ],
  })
  if (!outputFile) return

  await runDump(outputFile, true)
}

async function runDump(outputPath, isArchive) {
  if (!sourceUri.value || selectedCount.value === 0) return

  const entries = buildEntries(selectedCollections.value)
  // An archive is a single file: looping would overwrite it once per database
  // and report success for every one. startDumpArchive refuses that case, so
  // reaching here with more than one database is a bug.
  if (isArchive && entries.length > 1) {
    toast('An archive can only hold one database. Use Dump folder instead.', 'error')
    return
  }
  beginOperation(isArchive ? 'dump-archive' : 'dump-folder')

  for (const entry of entries) {
    if (aborting.value) break
    nextOpId()
    try {
      await invoke('mongodb_dump', {
        remoteUri: sourceUri.value,
        db: entry.db,
        collections: entry.collections,
        outputDir: outputPath,
        isArchive: isArchive,
        opId: currentOpId.value,
      })
      toast(`Dumped ${entry.db}: ${entry.collections.join(', ')}`, 'success')
    } catch (err) {
      if (handleOperationError(err, `Cancelled ${entry.db}`, `Dump failed for ${entry.db}`)) break
    }
  }

  resetOperationState()
}

function openRestoreConfirm(inputPath, isArchive, sourceDbs = []) {
  restoreConfirm.value = {
    show: true,
    inputPath,
    isArchive,
    entries: buildEntries(selectedCollections.value),
    sourceDbs,
    dropFirst: false,
  }
}

function cancelRestore() {
  restoreConfirm.value.show = false
}

async function confirmRestore() {
  const { inputPath, isArchive, sourceDbs, dropFirst } = restoreConfirm.value
  restoreConfirm.value.show = false
  await runRestore(inputPath, isArchive, restoreConfirm.value.entries, sourceDbs, dropFirst)
}

async function startRestoreFolder() {
  if (!destUri.value) return

  const inputDir = await open({
    directory: true,
    multiple: false,
    title: 'Select restore folder (mongodump output)',
  })
  if (!inputDir) return

  try {
    const sourceDbs = await invoke('scan_restore_folder', { path: inputDir })
    openRestoreConfirm(inputDir, false, sourceDbs)
  } catch (err) {
    toast(`Selected folder is not a valid dump: ${err}`, 'error')
  }
}

async function startRestoreFile() {
  if (!destUri.value) return

  const inputFile = await open({
    directory: false,
    multiple: false,
    title: 'Select restore archive',
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
    // User picked a folder representing a single DB (either direct or parent with one DB).
    // Tell mongorestore which DB to restore so it doesn't skip the files.
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
    // If nothing is selected and there are multiple DBs, restore every DB under the folder.
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
  if (!destUri.value) return

  beginOperation(isArchive ? 'restore-archive' : 'restore-folder')

  const hasSelection = entries.length > 0

  if (isArchive) {
    // Archives can contain many DBs; run one mongorestore with all selected namespaces.
    const includes = []
    for (const entry of entries) {
      for (const coll of entry.collections) {
        includes.push(`${entry.db}.${coll}`)
      }
    }

    try {
      await invoke('mongodb_restore_archive', {
        remoteUri: destUri.value,
        includes,
        inputPath,
        dropFirst,
        opId: currentOpId.value,
      })
      toast(hasSelection ? 'Restored selected collections from archive' : 'Restored archive', 'success')
    } catch (err) {
      handleOperationError(err, 'Cancelled archive restore', 'Archive restore failed')
    }
  } else {
    for (const job of folderRestoreJobs(entries, sourceDbs)) {
      if (aborting.value) break
      nextOpId()
      try {
        await invoke('mongodb_restore', {
          remoteUri: destUri.value,
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
  // Refresh the side we restored into so restored databases appear.
  await destSide.value.loadDatabases()
}

async function cancelOperation() {
  if (!syncing.value || !currentOpId.value || aborting.value) return
  aborting.value = true
  try {
    await invoke('mongodb_cancel', { opId: currentOpId.value })
  } catch (err) {
    toast(`Failed to cancel: ${err}`, 'error')
    aborting.value = false
  }
}

const listeners = useListenerGroup()

onMounted(async () => {
  await loadHost()
  await loadConfiguredDatabases()

  await listeners.listen('mongodb-sync-progress', (event) => {
    const p = event.payload
    if (currentOpId.value && p.opId && p.opId !== currentOpId.value) return
    syncProgress.value = {
      db: p.db || '',
      collection: p.collection || '',
      stage: p.stage || '',
      synced: p.synced || 0,
      total: p.total || 0,
      percent: p.percent !== undefined
        ? p.percent
        : (p.total > 0 ? Math.round((p.synced / p.total) * 100) : 0),
    }
  })

  await listeners.listen('mongodb-sync-cancelled', (event) => {
    const p = event.payload
    if (currentOpId.value && p.opId && p.opId !== currentOpId.value) return
    resetOperationState()
  })
})

watch(() => props.hostId, async () => {
  await loadHost()
  remote.reset()
  local.reset()
  selectedCollections.value = new Map()
  isRemoteToLocal.value = true
  await loadConfiguredDatabases()
})
</script>
