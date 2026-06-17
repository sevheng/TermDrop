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
        <button
          @click="startDump"
          :disabled="!canDumpRestore || syncing"
          class="py-2 text-xs font-medium rounded flex items-center justify-center gap-1.5 transition-colors"
          :class="canDumpRestore && !syncing
            ? 'bg-[#0e639c] hover:bg-[#1177bb] text-white'
            : 'bg-[#3c3c3c] text-[#6e6e6e] cursor-not-allowed'"
        >
          <Download v-if="!syncing" :size="12" />
          <Loader2 v-else :size="12" class="animate-spin" />
          {{ syncing && currentAction === 'dump' ? 'Dumping...' : 'Dump to folder' }}
        </button>
        <button
          @click="startRestore"
          :disabled="!canDumpRestore || syncing"
          class="py-2 text-xs font-medium rounded flex items-center justify-center gap-1.5 transition-colors"
          :class="canDumpRestore && !syncing
            ? 'bg-[#388a34] hover:bg-[#43a047] text-white'
            : 'bg-[#3c3c3c] text-[#6e6e6e] cursor-not-allowed'"
        >
          <Upload v-if="!syncing" :size="12" />
          <Loader2 v-else :size="12" class="animate-spin" />
          {{ syncing && currentAction === 'restore' ? 'Restoring...' : 'Restore from folder' }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'
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

const props = defineProps({
  hostId: { type: Number, required: true },
})

const host = ref(null)
const remoteDatabases = ref([])
const localDatabases = ref([])
const expandedRemoteDbs = ref(new Set())
const expandedLocalDbs = ref(new Set())
const selectedCollections = ref(new Map())
const loadingRemote = ref(false)
const loadingLocal = ref(false)
const syncing = ref(false)
const currentAction = ref('') // 'sync' | 'dump' | 'restore'
const dropFirst = ref(false)
const isRemoteToLocal = ref(true) // true = Remote→Local, false = Local→Remote
const currentOpId = ref('')
const aborting = ref(false)

const syncProgress = ref({
  db: '',
  collection: '',
  stage: '',
  synced: 0,
  total: 0,
  percent: 0,
})

function resetOperationState() {
  syncing.value = false
  currentAction.value = ''
  currentOpId.value = ''
  aborting.value = false
  syncProgress.value = { db: '', collection: '', stage: '', synced: 0, total: 0, percent: 0 }
}

const remoteUri = computed(() => host.value?.mongo_uri || '')
const localUri = computed(() => host.value?.mongo_local_uri || '')
const hasLocalUri = computed(() => !!localUri.value)

// Source/dest computed based on direction
const sourceUri = computed(() => isRemoteToLocal.value ? remoteUri.value : localUri.value)
const destUri = computed(() => isRemoteToLocal.value ? localUri.value : remoteUri.value)
const sourceDatabases = computed(() => isRemoteToLocal.value ? remoteDatabases.value : localDatabases.value)
const destDatabases = computed(() => isRemoteToLocal.value ? localDatabases.value : remoteDatabases.value)
const sourceExpandedDbs = computed(() => isRemoteToLocal.value ? expandedRemoteDbs.value : expandedLocalDbs.value)
const destExpandedDbs = computed(() => isRemoteToLocal.value ? expandedLocalDbs.value : expandedRemoteDbs.value)

const selectedCount = computed(() => {
  let count = 0
  for (const set of selectedCollections.value.values()) {
    count += set.size
  }
  return count
})

const canSync = computed(() => {
  return sourceUri.value && destUri.value && selectedCount.value > 0 && !syncing.value
})

const canDumpRestore = computed(() => {
  // Dump/Restore always operate on the actual Remote
  return remoteUri.value && selectedCount.value > 0 && !syncing.value
})

const syncButtonLabel = computed(() => {
  if (syncing.value) return 'Syncing...'
  const dir = isRemoteToLocal.value ? 'Remote → Local' : 'Local → Remote'
  return `Sync ${selectedCount.value} collection${selectedCount.value === 1 ? '' : 's'} (${dir})`
})

function isSelected(db, coll) {
  return selectedCollections.value.get(db)?.has(coll) || false
}

function dbSelectionState(db) {
  const selected = selectedCollections.value.get(db.name)
  if (!selected || selected.size === 0) return 'none'
  if (db.collections.length > 0 && selected.size === db.collections.length) return 'all'
  return 'some'
}

async function toggleDbSelection(db) {
  const state = dbSelectionState(db)
  const newMap = new Map(selectedCollections.value)

  if (state === 'all') {
    newMap.delete(db.name)
  } else {
    if (!sourceExpandedDbs.value.has(db.name)) {
      const newExpanded = new Set(sourceExpandedDbs.value)
      newExpanded.add(db.name)
      if (isRemoteToLocal.value) {
        expandedRemoteDbs.value = newExpanded
      } else {
        expandedLocalDbs.value = newExpanded
      }
    }
    if (db.collections.length === 0 && !db.loading) {
      await fetchSourceCollections(db)
    }
    newMap.set(db.name, new Set(db.collections))
  }
  selectedCollections.value = newMap
}

function toggleCollection(db, coll) {
  const set = selectedCollections.value.get(db) || new Set()
  const newSet = new Set(set)
  if (newSet.has(coll)) {
    newSet.delete(coll)
  } else {
    newSet.add(coll)
  }
  const newMap = new Map(selectedCollections.value)
  if (newSet.size === 0) {
    newMap.delete(db)
  } else {
    newMap.set(db, newSet)
  }
  selectedCollections.value = newMap
}

function clearDbSelection(db) {
  const newMap = new Map(selectedCollections.value)
  newMap.delete(db)
  selectedCollections.value = newMap
}

function clearSelection() {
  selectedCollections.value = new Map()
}

function toggleSourceDb(dbName) {
  const newSet = new Set(sourceExpandedDbs.value)
  if (newSet.has(dbName)) {
    newSet.delete(dbName)
  } else {
    newSet.add(dbName)
    const db = sourceDatabases.value.find(d => d.name === dbName)
    if (db && db.collections.length === 0 && !db.loading) {
      fetchSourceCollections(db)
    }
  }
  if (isRemoteToLocal.value) {
    expandedRemoteDbs.value = newSet
  } else {
    expandedLocalDbs.value = newSet
  }
}

function toggleDestDb(dbName) {
  const newSet = new Set(destExpandedDbs.value)
  if (newSet.has(dbName)) {
    newSet.delete(dbName)
  } else {
    newSet.add(dbName)
    const db = destDatabases.value.find(d => d.name === dbName)
    if (db && db.collections.length === 0 && !db.loading) {
      fetchDestCollections(db)
    }
  }
  if (isRemoteToLocal.value) {
    expandedLocalDbs.value = newSet
  } else {
    expandedRemoteDbs.value = newSet
  }
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

async function loadRemoteDatabases() {
  if (!remoteUri.value) return
  loadingRemote.value = true
  try {
    const dbNames = await invoke('mongodb_list_databases', { uri: remoteUri.value })
    remoteDatabases.value = dbNames.map(name => ({
      name,
      collections: [],
      loading: false,
    }))
  } catch (err) {
    toast('Failed to list remote databases: ' + err, 'error')
  } finally {
    loadingRemote.value = false
  }
}

async function loadLocalDatabases() {
  if (!localUri.value) return
  loadingLocal.value = true
  try {
    const dbNames = await invoke('mongodb_list_databases', { uri: localUri.value })
    localDatabases.value = dbNames.map(name => ({
      name,
      collections: [],
      loading: false,
    }))
  } catch (err) {
    toast('Failed to list local databases: ' + err, 'error')
  } finally {
    loadingLocal.value = false
  }
}

async function fetchSourceCollections(db) {
  db.loading = true
  try {
    const colls = await invoke('mongodb_list_collections', {
      uri: sourceUri.value,
      db: db.name,
    })
    db.collections = colls
  } catch (err) {
    toast(`Failed to list source collections for ${db.name}: ${err}`, 'error')
  } finally {
    db.loading = false
  }
}

async function fetchDestCollections(db) {
  db.loading = true
  try {
    const colls = await invoke('mongodb_list_collections', {
      uri: destUri.value,
      db: db.name,
    })
    db.collections = colls
  } catch (err) {
    toast(`Failed to list dest collections for ${db.name}: ${err}`, 'error')
  } finally {
    db.loading = false
  }
}

async function startSync() {
  if (!sourceUri.value || !destUri.value || selectedCount.value === 0) return

  const entries = []
  for (const [db, set] of selectedCollections.value) {
    entries.push({ db, collections: Array.from(set) })
  }

  syncing.value = true
  currentAction.value = 'sync'
  currentOpId.value = crypto.randomUUID()
  aborting.value = false
  syncProgress.value = { db: '', collection: '', stage: '', synced: 0, total: 0, percent: 0 }

  for (const entry of entries) {
    if (aborting.value || !currentOpId.value) break
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
      if (String(err).includes('cancelled')) {
        resetOperationState()
        toast(`Cancelled ${entry.db}`, 'info')
        break
      } else {
        toast(`Sync failed for ${entry.db}: ${err}`, 'error')
      }
    }
  }

  resetOperationState()
  // Refresh the destination side
  if (isRemoteToLocal.value) {
    await loadLocalDatabases()
  } else {
    await loadRemoteDatabases()
  }
}

async function startDump() {
  if (!remoteUri.value || selectedCount.value === 0) return

  const outputDir = await open({
    directory: true,
    multiple: false,
    title: 'Select dump output folder',
  })
  if (!outputDir) return

  const entries = []
  for (const [db, set] of selectedCollections.value) {
    entries.push({ db, collections: Array.from(set) })
  }

  syncing.value = true
  currentAction.value = 'dump'
  currentOpId.value = crypto.randomUUID()
  aborting.value = false
  syncProgress.value = { db: '', collection: '', stage: '', synced: 0, total: 0, percent: 0 }

  for (const entry of entries) {
    if (aborting.value || !currentOpId.value) break
    try {
      await invoke('mongodb_dump', {
        remoteUri: remoteUri.value,
        db: entry.db,
        collections: entry.collections,
        outputDir,
        opId: currentOpId.value,
      })
      toast(`Dumped ${entry.db}: ${entry.collections.join(', ')}`, 'success')
    } catch (err) {
      if (String(err).includes('cancelled')) {
        resetOperationState()
        toast(`Cancelled ${entry.db}`, 'info')
        break
      } else {
        toast(`Dump failed for ${entry.db}: ${err}`, 'error')
      }
    }
  }

  resetOperationState()
}

async function startRestore() {
  if (!remoteUri.value || selectedCount.value === 0) return

  const inputDir = await open({
    directory: true,
    multiple: false,
    title: 'Select restore folder (mongodump output)',
  })
  if (!inputDir) return

  const entries = []
  for (const [db, set] of selectedCollections.value) {
    entries.push({ db, collections: Array.from(set) })
  }

  syncing.value = true
  currentAction.value = 'restore'
  currentOpId.value = crypto.randomUUID()
  aborting.value = false
  syncProgress.value = { db: '', collection: '', stage: '', synced: 0, total: 0, percent: 0 }

  for (const entry of entries) {
    if (aborting.value || !currentOpId.value) break
    try {
      await invoke('mongodb_restore', {
        remoteUri: remoteUri.value,
        db: entry.db,
        collections: entry.collections,
        inputDir,
        opId: currentOpId.value,
      })
      toast(`Restored ${entry.db}: ${entry.collections.join(', ')}`, 'success')
    } catch (err) {
      if (String(err).includes('cancelled')) {
        resetOperationState()
        toast(`Cancelled ${entry.db}`, 'info')
        break
      } else {
        toast(`Restore failed for ${entry.db}: ${err}`, 'error')
      }
    }
  }

  resetOperationState()
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

let unlistenProgress = null
let unlistenCancelled = null

onMounted(async () => {
  await loadHost()
  if (remoteUri.value) {
    await loadRemoteDatabases()
  }
  if (localUri.value) {
    await loadLocalDatabases()
  }

  unlistenProgress = await listen('mongodb-sync-progress', (event) => {
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

  unlistenCancelled = await listen('mongodb-sync-cancelled', (event) => {
    const p = event.payload
    if (currentOpId.value && p.opId && p.opId !== currentOpId.value) return
    resetOperationState()
  })
})

onUnmounted(() => {
  if (unlistenProgress) unlistenProgress()
  if (unlistenCancelled) unlistenCancelled()
})

watch(() => props.hostId, async () => {
  await loadHost()
  remoteDatabases.value = []
  localDatabases.value = []
  selectedCollections.value = new Map()
  expandedRemoteDbs.value = new Set()
  expandedLocalDbs.value = new Set()
  isRemoteToLocal.value = true
  if (remoteUri.value) {
    await loadRemoteDatabases()
  }
  if (localUri.value) {
    await loadLocalDatabases()
  }
})

function toast(message, type = 'info') {
  window.dispatchEvent(new CustomEvent('app-toast', { detail: { message, type } }))
}
</script>
