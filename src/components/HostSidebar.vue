<template>
  <div class="h-full w-full flex flex-col">
    <!-- Header -->
    <div class="p-2 border-b border-line flex items-center justify-between">
      <h2 class="text-xs font-semibold text-ink">Hosts</h2>
      <div class="flex items-center gap-0.5">
        <button
          @click="toggleView"
          class="text-ink-2 hover:text-ink p-1"
          :title="viewMode === 'grouped' ? 'Switch to flat view' : 'Switch to grouped view'"
        >
          <component :is="viewMode === 'grouped' ? List : LayoutGrid" :size="12" />
        </button>
        <div class="relative" ref="importMenuRef">
          <button @click="showImportMenu = !showImportMenu" class="text-ink-2 hover:text-ink p-1" title="Import">
            <Download :size="12" />
          </button>
          <div
            v-if="showImportMenu"
            class="absolute left-0 top-full mt-1 bg-surface border border-line rounded shadow-xl z-50 min-w-[180px] py-1"
          >
            <button
              @click="importSshConfig(); showImportMenu = false"
              class="w-full text-left px-3 py-1.5 text-xs text-ink hover:bg-raised flex items-center gap-2"
            >
              <FileTerminal :size="12" />
              From ~/.ssh/config
            </button>
            <button
              @click="importHosts(); showImportMenu = false"
              class="w-full text-left px-3 py-1.5 text-xs text-ink hover:bg-raised flex items-center gap-2"
            >
              <Download :size="12" />
              From JSON file
            </button>
          </div>
        </div>
        <button @click="store.exportHosts" class="text-ink-2 hover:text-ink p-1" title="Export hosts">
          <Upload :size="12" />
        </button>
        <div class="relative" ref="addMenuRef">
          <button @click="showAddMenu = !showAddMenu" class="text-ink-2 hover:text-ink p-1" title="Add">
            <Plus :size="12" />
          </button>
          <div
            v-if="showAddMenu"
            class="absolute right-0 top-full mt-1 bg-surface border border-line rounded shadow-xl z-50 min-w-[140px] py-1"
          >
            <button
              @click="openModal(); showAddMenu = false"
              class="w-full text-left px-3 py-1.5 text-xs text-ink hover:bg-raised flex items-center gap-2"
            >
              <Server :size="12" />
              Host
            </button>
            <button
              @click="openMongoModal(); showAddMenu = false"
              class="w-full text-left px-3 py-1.5 text-xs text-ink hover:bg-raised flex items-center gap-2"
            >
              <Database :size="12" />
              MongoDB
            </button>
            <button
              @click="openRedisModal(); showAddMenu = false"
              class="w-full text-left px-3 py-1.5 text-xs text-ink hover:bg-raised flex items-center gap-2"
            >
              <Layers :size="12" class="text-redis" />
              Redis
            </button>
          </div>
        </div>
      </div>
    </div>

    <!-- Search -->
    <div class="px-2 py-1 border-b border-line">
      <div class="relative">
        <Search :size="12" class="absolute left-2 top-1/2 -translate-y-1/2 text-ink-3" />
        <input
          v-model="searchQuery"
          type="text"
          placeholder="Search hosts..."
          class="w-full bg-input border border-line rounded pl-6 pr-2 py-1 text-xs text-ink placeholder-ink-3 focus:outline-none focus:border-accent"
        />
      </div>
    </div>

    <div class="flex-1 overflow-y-auto py-1 px-1" @contextmenu.prevent="showEmptyMenu">
      <!-- Empty state -->
      <div v-if="displayHosts.length === 0" class="flex flex-col items-center justify-center py-8 text-ink-3">
        <Server :size="24" class="mb-2 opacity-50" />
        <p class="text-xs">
          {{ store.hosts.length === 0 ? 'No hosts yet' : 'No matching hosts' }}
        </p>
        <p v-if="store.hosts.length === 0" class="text-xs mt-1">
          Click + to add your first host
        </p>
      </div>

      <!-- Flat view -->
      <template v-if="viewMode === 'flat'">
        <HostRow
          v-for="host in displayHosts"
          :key="host.id"
          :host="host"
          :is-connected="isHostConnected(host.id)"
          :is-connecting="store.connectingHostId === host.id"
          @connect="connectHost(host.id)"
          @edit="editHost(host)"
          @delete="deleteHost(host)"
          @toggle-favorite="toggleFavorite(host)"
          @context-menu="showHostMenu"
        />
      </template>

      <!-- Grouped view -->
      <template v-else>
        <!-- Favorites section -->
        <div v-if="favoriteHosts.length > 0 && !searchQuery.trim()" class="mb-1">
          <div class="px-2 py-0.5 text-2xs font-semibold text-ink-3 uppercase tracking-wider dark:text-ink-3 flex items-center gap-1">
            <Star :size="10" class="text-warn" />
            Favorites
          </div>
          <HostRow
            v-for="host in favoriteHosts"
            :key="'fav-' + host.id"
            :host="host"
            :is-connected="isHostConnected(host.id)"
            :is-connecting="store.connectingHostId === host.id"
            @connect="connectHost(host.id)"
            @edit="editHost(host)"
            @delete="deleteHost(host)"
            @toggle-favorite="toggleFavorite(host)"
            @context-menu="showHostMenu"
          />
        </div>

        <!-- Grouped hosts -->
        <template v-for="(groupHosts, groupName) in groupedHosts" :key="groupName">
          <div class="mb-1">
            <div
              class="flex items-center justify-between border-l-2 pl-2 pr-2 py-0.5 rounded-r cursor-pointer select-none hover:bg-raised"
              :class="[
                groupAccentClass(groupName),
                dragOverGroup === groupName ? 'ring-1 ring-accent' : '',
              ]"
              @click="toggleGroup(groupName)"
              @contextmenu.prevent.stop="showGroupMenu($event, groupName)"
              @dragover.prevent="dragOverGroup = groupName"
              @dragleave="dragOverGroup = null"
              @drop="onGroupDrop($event, groupName)"
            >
              <span class="flex items-center gap-1 text-2xs font-semibold text-ink-2">
                <component :is="collapsedGroups.has(groupName) ? Folder : FolderOpen" :size="10" />
                {{ groupName || 'Ungrouped' }}
              </span>
              <span class="text-2xs text-ink-3">{{ groupHosts.length }}</span>
            </div>
            <div v-show="!collapsedGroups.has(groupName)" class="pl-1">
              <HostRow
                v-for="host in groupHosts"
                :key="host.id"
                :host="host"
                :is-connected="isHostConnected(host.id)"
                :is-connecting="store.connectingHostId === host.id"
                @connect="connectHost(host.id)"
                @edit="editHost(host)"
                @delete="deleteHost(host)"
                @toggle-favorite="toggleFavorite(host)"
                @drag-end="dragOverGroup = null"
                @context-menu="showHostMenu"
              />
            </div>
          </div>
        </template>
      </template>
    </div>

    <!-- Unified Context Menu -->
    <div
      v-if="contextMenu.show"
      ref="contextMenuEl"
      class="fixed bg-surface border border-line rounded shadow-lg py-1 z-50 min-w-[10rem] max-h-[calc(100vh-16px)] overflow-y-auto"
      :style="{ top: contextMenu.y + 'px', left: contextMenu.x + 'px' }"
    >
      <!-- Host menu -->
      <template v-if="contextMenu.type === 'host'">
        <button @click="menuAction(() => activateHost(contextMenu.data))" class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-ink hover:bg-raised">
          <Zap :size="12" class="text-accent" />
          {{ activateVerb(contextMenu.data) }}
        </button>
        <button @click="menuAction(() => editHost(contextMenu.data))" class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-ink hover:bg-raised">
          <Pencil :size="12" class="text-ink-3" />
          Edit
        </button>
        <button @click="menuAction(() => toggleFavorite(contextMenu.data))" class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-ink hover:bg-raised">
          <Star :size="12" class="text-warn" />
          {{ contextMenu.data.favorite ? 'Unfavorite' : 'Favorite' }}
        </button>
        <div class="border-t border-line my-0.5"></div>
        <button @click="menuAction(() => deleteHost(contextMenu.data))" class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-bad hover:bg-raised">
          <Trash2 :size="12" />
          Delete
        </button>
        <div v-if="viewMode === 'grouped' && allGroupNames.length > 0" class="border-t border-line my-0.5"></div>
        <div v-if="viewMode === 'grouped' && allGroupNames.length > 0" class="px-3 py-0.5 text-2xs text-ink-3">Move to</div>
        <button
          v-for="g in allGroupNames"
          :key="g"
          @click="menuAction(() => moveHostToGroup(contextMenu.data.id, g))"
          class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-ink-2 hover:bg-raised"
        >
          <Folder :size="10" class="text-ink-3" />
          {{ g || 'Ungrouped' }}
        </button>
      </template>

      <!-- Group menu -->
      <template v-if="contextMenu.type === 'group'">
        <button @click="menuAction(addHostToGroup)" class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-ink hover:bg-raised">
          <Plus :size="12" class="text-good" />
          Add Host
        </button>
        <button @click="menuAction(startRenameGroup)" class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-ink hover:bg-raised">
          <Pencil :size="12" class="text-ink-3" />
          Rename
        </button>
        <button @click="menuAction(deleteGroup)" class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-bad hover:bg-raised">
          <Trash2 :size="12" />
          Delete group
        </button>
      </template>

      <!-- Empty area menu -->
      <template v-if="contextMenu.type === 'empty'">
        <button
          v-if="viewMode === 'grouped'"
          @click="menuAction(createGroupFromMenu)"
          class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-ink hover:bg-raised"
        >
          <FolderPlus :size="12" class="text-accent" />
          New Group
        </button>
        <button @click="menuAction(() => { openModal(); })" class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-ink hover:bg-raised">
          <Plus :size="12" class="text-good" />
          Add Host
        </button>
      </template>
    </div>

    <HostModal
      :show="showModal"
      :host="editingHost"
      @close="showModal = false"
      @save="handleSave"
    />

    <MongoDbModal
      :show="showMongoModal"
      :host="editingHost"
      @close="showMongoModal = false"
      @save="handleMongoSave"
    />

    <RedisModal
      :show="showRedisModal"
      :host="editingHost"
      @close="showRedisModal = false"
      @save="handleRedisSave"
    />

    <GroupModal
      :show="showGroupModal"
      :mode="groupModalMode"
      :existing-names="allGroupNames.filter(g => g !== groupModalCurrentName)"
      :current-name="groupModalCurrentName"
      @close="showGroupModal = false"
      @save="handleGroupModalSave"
    />

    <ConfirmDialog
      :show="confirmDialog.show"
      :title="confirmDialog.title"
      :message="confirmDialog.message"
      :danger="confirmDialog.danger"
      confirm-text="Delete"
      @confirm="confirmDialog.onConfirm"
      @cancel="confirmDialog.show = false"
    />

    <input ref="importInput" type="file" accept=".json" class="hidden" @change="onImportFileSelected" />

    <SshConfigImportDialog ref="sshImportRef" />
  </div>
</template>

<script setup>
import { ref, onMounted, computed, watch, onUnmounted } from 'vue'
import { useConnectionStore } from '../stores/connection.js'
import {
  Plus, Server, Database, Layers, Search, Upload, Download, FileTerminal,
  Folder, FolderOpen, FolderPlus,
  List, LayoutGrid, Star,
  Zap, Pencil, Trash2,
} from 'lucide-vue-next'
import HostModal from './HostModal.vue'
import MongoDbModal from './MongoDbModal.vue'
import RedisModal from './RedisModal.vue'
import GroupModal from './GroupModal.vue'
import ConfirmDialog from './ConfirmDialog.vue'
import HostRow from './HostRow.vue'
import SshConfigImportDialog from './SshConfigImportDialog.vue'
import { toast } from '../utils/toast.js'
import { parseHostsFile, normalizeImportHost, summarizeImport } from '../utils/hostImport.js'
import { splitMongoUri } from '../utils/mongoUri.js'
import { splitRedisUri } from '../utils/redisUri.js'
import { hostKind, HOST_KIND, activateVerb } from '../utils/hostKind.js'
import { groupAccentClass } from '../utils/groupAccent.js'
import { invoke } from '../utils/invoke.js'
import { useConfirmDialog } from '../composables/useConfirmDialog.js'
import { useContextMenu } from '../composables/useContextMenu.js'

const store = useConnectionStore()

const showModal = ref(false)
const showMongoModal = ref(false)
const showRedisModal = ref(false)
const editingHost = ref(null)
const searchQuery = ref('')
const debouncedQuery = ref('')
const collapsedGroups = ref(new Set())

// Debounce search input to reduce computed recalculations
let searchDebounceTimer = null
watch(searchQuery, (val) => {
  if (searchDebounceTimer) clearTimeout(searchDebounceTimer)
  searchDebounceTimer = setTimeout(() => {
    debouncedQuery.value = val
  }, 200)
}, { immediate: true })
const importInput = ref(null)
const viewMode = ref(localStorage.getItem('host-view-mode') || 'grouped')
const dragOverGroup = ref(null)
const customGroups = ref(new Set(JSON.parse(localStorage.getItem('host-custom-groups') || '[]')))

const pendingGroupForNewHost = ref(null)

const showGroupModal = ref(false)
const groupModalMode = ref('create')

const sshImportRef = ref(null)
const showImportMenu = ref(false)
const importMenuRef = ref(null)
const showAddMenu = ref(false)
const addMenuRef = ref(null)
const groupModalCurrentName = ref('')

const { confirmDialog, openConfirm } = useConfirmDialog()

const filteredHosts = computed(() => {
  const q = debouncedQuery.value.trim().toLowerCase()
  if (!q) return store.hosts
  return store.hosts.filter(h =>
    h.name.toLowerCase().includes(q) ||
    h.host.toLowerCase().includes(q) ||
    h.username.toLowerCase().includes(q) ||
    (h.group && h.group.toLowerCase().includes(q))
  )
})

const nonFavoriteHosts = computed(() => filteredHosts.value.filter(h => !h.favorite))

const displayHosts = computed(() => {
  if (viewMode.value === 'flat') return filteredHosts.value
  return nonFavoriteHosts.value
})

const favoriteHosts = computed(() => {
  return filteredHosts.value.filter(h => h.favorite)
})

const allGroupNames = computed(() => {
  const groups = new Set()
  for (const h of store.hosts) {
    groups.add(h.group || '')
  }
  for (const g of customGroups.value) {
    groups.add(g)
  }
  return [...groups].sort((a, b) => {
    if (!a) return 1
    if (!b) return -1
    return a.localeCompare(b)
  })
})

const groupedHosts = computed(() => {
  const groups = {}
  for (const host of nonFavoriteHosts.value) {
    const g = host.group || ''
    if (!groups[g]) groups[g] = []
    groups[g].push(host)
  }
  for (const g of customGroups.value) {
    if (!(g in groups)) groups[g] = []
  }
  const sorted = {}
  const keys = Object.keys(groups).sort((a, b) => {
    if (!a) return 1
    if (!b) return -1
    return a.localeCompare(b)
  })
  for (const k of keys) {
    sorted[k] = groups[k]
  }
  return sorted
})


function toggleView() {
  viewMode.value = viewMode.value === 'grouped' ? 'flat' : 'grouped'
  localStorage.setItem('host-view-mode', viewMode.value)
}

function isHostConnected(hostId) {
  return store.tabs.some(t => t.hostId === hostId)
}

function toggleGroup(name) {
  const set = new Set(collapsedGroups.value)
  if (set.has(name)) set.delete(name)
  else set.add(name)
  collapsedGroups.value = set
}

async function toggleFavorite(host) {
  await store.setHostFavorite(host.id, !host.favorite)
}

const contextMenuEl = ref(null)
const {
  contextMenu,
  openContextMenu: openMenuAt,
  closeContextMenu: hideContextMenu,
} = useContextMenu(contextMenuEl, { type: '', data: null })

// Open the menu at the cursor; it is shifted back inside the window once
// rendered so tall menus near the bottom/right edge are not clipped.
function openContextMenu(event, type, data) {
  openMenuAt(event, { type, data })
}

function showGroupMenu(event, groupName) {
  event.preventDefault()
  event.stopPropagation()
  openContextMenu(event, 'group', groupName)
}

function showEmptyMenu(event) {
  openContextMenu(event, 'empty', null)
}

function showHostMenu(event, host) {
  event.stopPropagation()
  openContextMenu(event, 'host', host)
}

function menuAction(fn) {
  hideContextMenu()
  fn()
}

function addHostToGroup() {
  pendingGroupForNewHost.value = contextMenu.value.data
  openModal()
}

async function moveHostToGroup(hostId, groupName) {
  await store.setHostGroup(hostId, groupName)
}

function startRenameGroup() {
  groupModalMode.value = 'rename'
  groupModalCurrentName.value = contextMenu.value.data
  showGroupModal.value = true
}

async function deleteGroup() {
  const name = contextMenu.value.data
  if (!name) return
  openConfirm({
    title: 'Delete Group',
    message: `Delete group "${name}"? Hosts will be moved to Ungrouped.`,
    danger: true,
    onConfirm: async () => {
      await store.deleteGroup(name)
      customGroups.value = new Set([...customGroups.value].filter(g => g !== name))
      localStorage.setItem('host-custom-groups', JSON.stringify([...customGroups.value]))
    },
  })
}

async function onGroupDrop(event, groupName) {
  dragOverGroup.value = null
  const data = event.dataTransfer.getData('application/json')
  if (!data) return
  try {
    const { hostId } = JSON.parse(data)
    await store.setHostGroup(hostId, groupName)
  } catch (err) {
    console.error('Drop failed:', err)
  }
}

function createGroupFromMenu() {
  groupModalMode.value = 'create'
  groupModalCurrentName.value = ''
  showGroupModal.value = true
}

async function handleGroupModalSave(name) {
  showGroupModal.value = false
  if (groupModalMode.value === 'create') {
    customGroups.value = new Set([...customGroups.value, name])
    localStorage.setItem('host-custom-groups', JSON.stringify([...customGroups.value]))
    collapsedGroups.value = new Set([...collapsedGroups.value].filter(g => g !== name))
  } else if (groupModalMode.value === 'rename') {
    const oldName = groupModalCurrentName.value
    if (name !== oldName) {
      await store.renameGroup(oldName, name)
      // Update customGroups: remove old name, add new name
      const updated = new Set([...customGroups.value].filter(g => g !== oldName))
      updated.add(name)
      customGroups.value = updated
      localStorage.setItem('host-custom-groups', JSON.stringify([...updated]))
      // Preserve collapsed state under new name
      if (collapsedGroups.value.has(oldName)) {
        const newCollapsed = new Set([...collapsedGroups.value].filter(g => g !== oldName))
        newCollapsed.add(name)
        collapsedGroups.value = newCollapsed
      }
    }
  }
}

function onWindowClick(e) {
  hideContextMenu()
  if (showImportMenu.value && importMenuRef.value && !importMenuRef.value.contains(e.target)) {
    showImportMenu.value = false
  }
  if (showAddMenu.value && addMenuRef.value && !addMenuRef.value.contains(e.target)) {
    showAddMenu.value = false
  }
}

onMounted(() => {
  store.loadHosts()
  window.addEventListener('click', onWindowClick)
})

onUnmounted(() => {
  window.removeEventListener('click', onWindowClick)
  if (searchDebounceTimer) clearTimeout(searchDebounceTimer)
})

function openModal() {
  editingHost.value = null
  showModal.value = true
}

function openMongoModal() {
  editingHost.value = null
  showMongoModal.value = true
}

function openRedisModal() {
  editingHost.value = null
  showRedisModal.value = true
}

/** Each kind edits in its own dialog, chosen by what the row actually is. */
function editHost(host) {
  editingHost.value = host
  const kind = hostKind(host)
  showMongoModal.value = kind === HOST_KIND.MONGODB
  showRedisModal.value = kind === HOST_KIND.REDIS
  showModal.value = kind === HOST_KIND.SSH
}

/** Store a host password, reporting failure as a toast rather than throwing. */
async function savePassword(hostId, password) {
  if (!password) return
  await store.storePassword(hostId, password).catch((err) => {
    console.warn('Failed to store password:', err)
    toast('Password could not be saved: ' + err, 'error')
  })
}

async function handleSave({ id, hostData, password }) {
  if (id) {
    await store.updateHost(id, hostData)
    await savePassword(id, password)
  } else {
    const newId = await store.addHost(hostData)
    await savePassword(newId, password)
    if (pendingGroupForNewHost.value !== null) {
      await store.setHostGroup(newId, pendingGroupForNewHost.value)
      pendingGroupForNewHost.value = null
    }
  }
  showModal.value = false
  await store.loadHosts()
}

async function handleMongoSave({ id, name, mongo_uri }) {
  // The password is kept in the keyring, never in the database, so split it out
  // before the row is written.
  const { uri, password } = splitMongoUri(mongo_uri)

  const hostData = {
    name,
    host: '',
    port: 0,
    username: '',
    auth_type: 'password',
    key_path: null,
    group: null,
    favorite: null,
    mongo_uri: uri,
    mongo_local_uri: null,
  }

  try {
    const hostId = id || (await store.addHost(hostData))
    if (id) await store.updateHost(id, hostData)

    // Only ever store a password that was actually supplied. On edit the field
    // renders empty because the stored URI has none, and an empty field must
    // not be read as "delete the stored password".
    if (password) {
      await invoke('mongodb_store_secret', { hostId, password })
    }
  } catch (err) {
    toast(`Failed to save MongoDB connection: ${err}`, 'error')
    return
  }

  showMongoModal.value = false
  await store.loadHosts()
}

async function handleRedisSave({ id, name, redis_uri, redis_tunnel_host_id }) {
  // Same rule as MongoDB: the password lives in the keyring, so split it out
  // before the row is written and never let it reach SQLite.
  const { uri, password } = splitRedisUri(redis_uri)

  const hostData = {
    name,
    host: '',
    port: 0,
    username: '',
    auth_type: 'password',
    key_path: null,
    group: null,
    favorite: null,
    mongo_uri: null,
    mongo_local_uri: null,
    redis_uri: uri,
    redis_tunnel_host_id: redis_tunnel_host_id ?? null,
  }

  try {
    const hostId = id || (await store.addHost(hostData))
    if (id) await store.updateHost(id, hostData)

    // Only ever store a password that was actually supplied: on edit the field
    // renders empty because the stored URI has none, and an empty field must
    // not be read as "delete the stored password".
    if (password) {
      await invoke('redis_store_secret', { hostId, password })
    }
  } catch (err) {
    toast(`Failed to save Redis connection: ${err}`, 'error')
    return
  }

  showRedisModal.value = false
  await store.loadHosts()
}

/** Open a datastore panel, or connect a shell. */
async function activateHost(host) {
  const kind = hostKind(host)
  if (kind !== HOST_KIND.SSH) {
    store.openServiceTab(host.id, kind)
    return
  }
  try {
    await store.connect(host.id)
  } catch (err) {
    console.error('Connection failed:', err)
  }
}

async function connectHost(id) {
  await activateHost(store.hosts.find(h => h.id === id))
}

function deleteHost(host) {
  openConfirm({
    title: 'Delete Host',
    // A datastore row has no SSH host, and "(  )" reads as a bug.
    message: `Delete "${host.name}"${host.host ? ` (${host.host})` : ''}? This cannot be undone.`,
    danger: true,
    onConfirm: async () => {
      const openTab = store.tabs.find(t => t.hostId === host.id)
      if (openTab) {
        await store.disconnect(openTab.id)
      }
      await store.removeHost(host.id)
    },
  })
}

function importSshConfig() {
  sshImportRef.value?.open()
}

function importHosts() {
  importInput.value?.click()
}

const MAX_IMPORT_BYTES = 5 * 1024 * 1024

async function onImportFileSelected(event) {
  const file = event.target.files[0]
  event.target.value = ''
  if (!file) return
  if (file.size > MAX_IMPORT_BYTES) {
    toast('That file is too large to be a host export', 'error')
    return
  }
  try {
    const text = await file.text()
    const entries = parseHostsFile(text).map((raw) => ({
      host: normalizeImportHost(raw),
      replace_id: null,
    }))
    const summary = await store.importHosts(entries)
    const { message, type } = summarizeImport(summary)
    toast(message, type)
  } catch (err) {
    toast('Import failed: ' + err, 'error')
  }
}
</script>