<template>
  <div class="h-full flex flex-col bg-canvas">
    <!-- Toolbar -->
    <div class="flex items-center justify-between px-4 py-2.5 border-b border-line shrink-0 bg-surface">
      <div class="flex items-center gap-2 min-w-0">
        <Database :size="14" class="text-mongo shrink-0" />
        <span class="text-sm font-medium text-ink truncate">{{ connectionName }}</span>
        <span
          class="text-2xs text-ink-2 truncate max-w-[18rem]"
          :title="connectionDisplay"
        >
          {{ connectionDisplay }}
        </span>
      </div>

      <div class="flex items-center gap-1.5 shrink-0">
        <button
          @click="loadDatabases"
          :disabled="loading"
          class="p-1.5 rounded text-ink-2 hover:text-ink hover:bg-raised disabled:opacity-40"
          title="Refresh databases"
        >
          <RefreshCw :size="13" :class="loading ? 'animate-spin' : ''" />
        </button>

        <div class="w-px h-4 bg-input mx-1" />

        <button
          @click="backup.backupFolder"
          :disabled="!canBackup"
          class="px-2.5 py-1 text-xs rounded flex items-center gap-1.5 transition-colors"
          :class="canBackup
            ? 'bg-accent-solid hover:bg-accent-solid-hover text-white'
            : 'bg-input text-ink-3 cursor-not-allowed'"
          :title="selectedCount === 0 ? 'Select collections to back up' : 'Back up to a folder'"
        >
          <Download :size="12" />
          Backup
        </button>
        <button
          @click="backup.backupArchive"
          :disabled="!canBackup"
          class="px-2 py-1 text-xs rounded text-ink hover:bg-raised disabled:opacity-40 disabled:cursor-not-allowed"
          title="Back up to a single archive file"
        >
          Archive
        </button>

        <div class="w-px h-4 bg-input mx-1" />

        <button
          @click="backup.restoreFolder"
          :disabled="backup.busy.value"
          class="px-2.5 py-1 text-xs rounded flex items-center gap-1.5 bg-mongo-solid hover:bg-mongo-solid-hover text-white disabled:opacity-40 disabled:cursor-not-allowed"
          title="Restore from a backup folder"
        >
          <Upload :size="12" />
          Restore
        </button>
        <button
          @click="backup.restoreFile"
          :disabled="backup.busy.value"
          class="px-2 py-1 text-xs rounded text-ink hover:bg-raised disabled:opacity-40 disabled:cursor-not-allowed"
          title="Restore from an archive file"
        >
          From file
        </button>
      </div>
    </div>

    <!-- Progress, while a backup or restore runs -->
    <div v-if="backup.busy.value" class="px-4 py-2 border-b border-line shrink-0 bg-surface space-y-1">
      <div class="flex items-center justify-between text-2xs text-ink-3">
        <span>{{ backup.progress.value.stage }} {{ backup.progress.value.collection }}</span>
        <button @click="backup.cancel" class="text-bad hover:text-bad underline">
          Cancel
        </button>
      </div>
      <div class="h-1.5 bg-input rounded-full overflow-hidden">
        <div
          class="h-full bg-accent rounded-full transition-all duration-300"
          :style="{ width: backup.progress.value.percent + '%' }"
        />
      </div>
      <div class="flex justify-between text-2xs text-ink-3">
        <span>
          {{ backup.progress.value.detail
            || `${backup.progress.value.synced} / ${backup.progress.value.total}` }}
        </span>
        <span>{{ backup.progress.value.percent }}%</span>
      </div>
    </div>

    <!-- Tree | documents -->
    <div class="flex-1 flex overflow-hidden">
      <div class="w-72 shrink-0 flex flex-col border-r border-line">
        <div class="flex items-center justify-between px-3 py-1.5 border-b border-line shrink-0">
          <span class="text-2xs font-medium text-ink-2 uppercase tracking-wider">
            Databases
          </span>
          <span v-if="selectedCount > 0" class="text-2xs text-accent-soft">
            {{ selectedCount }} selected
            <button
              @click="clearSelection"
              class="ml-1 text-ink-2 hover:text-ink underline"
            >
              clear
            </button>
          </span>
        </div>

        <DbTree
          :databases="databases"
          :expanded-dbs="expandedDbs"
          :selected-collections="selectedCollections"
          :selectable="true"
          :browsable="true"
          :active-collection="viewing.collection"
          :active-db="viewing.db"
          :loading="loading"
          @toggle-db="toggleDb"
          @toggle-db-selection="toggleDbSelection"
          @toggle-collection="toggleCollection"
          @open-collection="openCollection"
        />
      </div>

      <div class="flex-1 min-w-0">
        <MongoDocumentsView
          v-if="viewing.collection"
          :key="`${viewing.db}.${viewing.collection}`"
          :host-id="hostId"
          :db="viewing.db"
          :collection="viewing.collection"
        />
        <div v-else class="h-full flex flex-col items-center justify-center text-ink-3">
          <FileSearch :size="22" class="mb-2 opacity-50" />
          <p class="text-xs">Select a collection to view its documents</p>
          <p class="text-2xs mt-1 text-ink-3">
            Tick collections to include them in a backup
          </p>
        </div>
      </div>
    </div>

    <MongoRestoreDialog
      :state="backup.restoreConfirm.value"
      :connection-name="connectionName"
      @confirm="backup.confirmRestore"
      @cancel="backup.cancelRestore"
      @update:drop-first="backup.restoreConfirm.value.dropFirst = $event"
    />
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted } from 'vue'
import { Database, RefreshCw, Download, Upload, FileSearch } from 'lucide-vue-next'
import DbTree from './DbTree.vue'
import MongoDocumentsView from './MongoDocumentsView.vue'
import MongoRestoreDialog from './MongoRestoreDialog.vue'
import { invoke } from '../utils/invoke.js'
import { toast } from '../utils/toast.js'
import { mongoDisplayUri } from '../utils/mongoDisplay.js'
import {
  toggleCollection as toggleCollectionIn,
  withoutDb,
  withAllCollections,
  dbSelectionState,
  countSelected,
} from '../utils/mongoSelection.js'
import { useMongoConnection, fetchCollections } from '../composables/useMongoConnection.js'
import { useMongoBackup } from '../composables/useMongoBackup.js'
import { useListenerGroup } from '../composables/useListenerGroup.js'

const props = defineProps({
  hostId: { type: Number, required: true },
})

const host = ref(null)
const connectionName = computed(() => host.value?.name || 'MongoDB')
const connectionDisplay = computed(() => mongoDisplayUri(host.value?.mongo_uri))

const hostId = computed(() => (host.value?.mongo_uri ? props.hostId : null))
const connection = useMongoConnection(hostId)
const { databases, expandedDbs, loading } = connection
const loadDatabases = () => connection.loadDatabases()

/** Collections ticked for the next backup: Map<db, Set<collection>>. */
const selectedCollections = ref(new Map())
const selectedCount = computed(() => countSelected(selectedCollections.value))

/** The collection whose documents fill the right pane. */
const viewing = ref({ db: '', collection: '' })

const backup = useMongoBackup(hostId, selectedCollections, loadDatabases)
const canBackup = computed(() => !!hostId.value && selectedCount.value > 0 && !backup.busy.value)

function openCollection(db, collection) {
  viewing.value = { db, collection }
}

function toggleCollection(db, coll) {
  selectedCollections.value = toggleCollectionIn(selectedCollections.value, db, coll)
}

async function toggleDb(dbName) {
  const next = new Set(expandedDbs.value)
  if (next.has(dbName)) {
    next.delete(dbName)
  } else {
    next.add(dbName)
    const db = databases.value.find(d => d.name === dbName)
    if (db && db.collections.length === 0 && !db.loading) {
      fetchCollections(props.hostId, db)
    }
  }
  expandedDbs.value = next
}

async function toggleDbSelection(db) {
  if (dbSelectionState(selectedCollections.value, db) === 'all') {
    selectedCollections.value = withoutDb(selectedCollections.value, db.name)
    return
  }

  // Selecting a whole database needs its collections, which load lazily.
  if (db.collections.length === 0 && !db.loading) {
    expandedDbs.value = new Set(expandedDbs.value).add(db.name)
    await fetchCollections(props.hostId, db)
  }
  selectedCollections.value = withAllCollections(selectedCollections.value, db)
}

function clearSelection() {
  selectedCollections.value = new Map()
}

async function loadHost() {
  try {
    host.value = await invoke('get_host_by_id', { id: props.hostId })
  } catch (err) {
    toast('Failed to load host: ' + err, 'error')
  }
}

const listeners = useListenerGroup()

onMounted(async () => {
  await loadHost()
  await loadDatabases()

  // Both events are shared by backup and restore.
  await listeners.listen('mongodb-sync-progress', e => backup.applyProgress(e.payload))
  await listeners.listen('mongodb-sync-cancelled', e => backup.applyCancelled(e.payload))
})

watch(
  () => props.hostId,
  async () => {
    connection.reset()
    selectedCollections.value = new Map()
    viewing.value = { db: '', collection: '' }
    await loadHost()
    await loadDatabases()
  },
)
</script>
