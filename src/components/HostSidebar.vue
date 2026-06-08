<template>
  <div class="h-full w-full flex flex-col">
    <!-- Header -->
    <div class="p-2 border-b border-[#3c3c3c] flex items-center justify-between">
      <h2 class="text-xs font-semibold text-[#cccccc]">Hosts</h2>
      <div class="flex items-center gap-0.5">
        <button
          @click="toggleView"
          class="text-[#858585] hover:text-[#cccccc] p-1"
          :title="viewMode === 'grouped' ? 'Switch to flat view' : 'Switch to grouped view'"
        >
          <component :is="viewMode === 'grouped' ? List : LayoutGrid" :size="12" />
        </button>
        <button @click="importHosts" class="text-[#858585] hover:text-[#cccccc] p-1" title="Import hosts">
          <Download :size="12" />
        </button>
        <button @click="store.exportHosts" class="text-[#858585] hover:text-[#cccccc] p-1" title="Export hosts">
          <Upload :size="12" />
        </button>
        <button @click="openModal()" class="text-[#858585] hover:text-[#cccccc] p-1" title="Add host">
          <Plus :size="12" />
        </button>
      </div>
    </div>

    <!-- Search -->
    <div class="px-2 py-1 border-b border-[#3c3c3c]">
      <div class="relative">
        <Search :size="12" class="absolute left-2 top-1/2 -translate-y-1/2 text-[#6e6e6e]" />
        <input
          v-model="searchQuery"
          type="text"
          placeholder="Search hosts..."
          class="w-full bg-[#3c3c3c] border border-[#3c3c3c] rounded pl-6 pr-2 py-1 text-xs text-[#cccccc] placeholder-[#6e6e6e] focus:outline-none focus:border-[#007acc]"
        />
      </div>
    </div>

    <div class="flex-1 overflow-y-auto py-1 px-1" @contextmenu.prevent="showEmptyMenu">
      <!-- Empty state -->
      <div v-if="displayHosts.length === 0" class="flex flex-col items-center justify-center py-8 text-[#6e6e6e]">
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
          <div class="px-2 py-0.5 text-[10px] font-semibold text-gray-400 uppercase tracking-wider dark:text-gray-500 flex items-center gap-1">
            <Star :size="10" class="text-[#cca700]" />
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
              class="flex items-center justify-between px-2 py-0.5 rounded cursor-pointer select-none"
              :class="[groupColorClass(groupName), dragOverGroup === groupName ? 'ring-1 ring-blue-400' : '']"
              @click="toggleGroup(groupName)"
              @contextmenu.prevent.stop="showGroupMenu($event, groupName)"
              @dragover.prevent="dragOverGroup = groupName"
              @dragleave="dragOverGroup = null"
              @drop="onGroupDrop($event, groupName)"
            >
              <span class="flex items-center gap-1 text-[10px] font-semibold text-[#858585]">
                <component :is="collapsedGroups.has(groupName) ? Folder : FolderOpen" :size="10" />
                {{ groupName || 'Ungrouped' }}
              </span>
              <span class="text-[10px] text-[#6e6e6e]">{{ groupHosts.length }}</span>
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
                @drag-start="draggingHost = true"
                @drag-end="draggingHost = false; dragOverGroup = null"
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
      class="fixed bg-[#252526] border border-[#3c3c3c] rounded shadow-lg py-1 z-50 min-w-[10rem]"
      :style="{ top: contextMenu.y + 'px', left: contextMenu.x + 'px' }"
    >
      <!-- Host menu -->
      <template v-if="contextMenu.type === 'host'">
        <button @click="menuAction(() => connectHost(contextMenu.data.id))" class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-[#cccccc] hover:bg-[#2a2d2e]">
          <Zap :size="12" class="text-[#007acc]" />
          Connect
        </button>
        <button @click="menuAction(() => editHost(contextMenu.data))" class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-[#cccccc] hover:bg-[#2a2d2e]">
          <Pencil :size="12" class="text-gray-400" />
          Edit
        </button>
        <button @click="menuAction(() => toggleFavorite(contextMenu.data))" class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-[#cccccc] hover:bg-[#2a2d2e]">
          <Star :size="12" class="text-[#cca700]" />
          {{ contextMenu.data.favorite ? 'Unfavorite' : 'Favorite' }}
        </button>
        <div class="border-t border-[#3c3c3c] my-0.5"></div>
        <button @click="menuAction(() => deleteHost(contextMenu.data))" class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-[#f44336] hover:bg-[#2a2d2e]">
          <Trash2 :size="12" />
          Delete
        </button>
        <div v-if="viewMode === 'grouped' && allGroupNames.length > 0" class="border-t border-[#3c3c3c] my-0.5"></div>
        <div v-if="viewMode === 'grouped' && allGroupNames.length > 0" class="px-3 py-0.5 text-[10px] text-[#6e6e6e]">Move to</div>
        <button
          v-for="g in allGroupNames"
          :key="g"
          @click="menuAction(() => moveHostToGroup(contextMenu.data.id, g))"
          class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-[#858585] hover:bg-[#2a2d2e]"
        >
          <Folder :size="10" class="text-gray-400" />
          {{ g || 'Ungrouped' }}
        </button>
      </template>

      <!-- Group menu -->
      <template v-if="contextMenu.type === 'group'">
        <button @click="menuAction(addHostToGroup)" class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-[#cccccc] hover:bg-[#2a2d2e]">
          <Plus :size="12" class="text-[#89d185]" />
          Add Host
        </button>
        <button @click="menuAction(startRenameGroup)" class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-[#cccccc] hover:bg-[#2a2d2e]">
          <Pencil :size="12" class="text-gray-400" />
          Rename
        </button>
        <button @click="menuAction(deleteGroup)" class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-[#f44336] hover:bg-[#2a2d2e]">
          <Trash2 :size="12" />
          Delete group
        </button>
      </template>

      <!-- Empty area menu -->
      <template v-if="contextMenu.type === 'empty'">
        <button
          v-if="viewMode === 'grouped'"
          @click="menuAction(createGroupFromMenu)"
          class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-[#cccccc] hover:bg-[#2a2d2e]"
        >
          <FolderPlus :size="12" class="text-[#007acc]" />
          New Group
        </button>
        <button @click="menuAction(() => { openModal(); })" class="flex items-center gap-2 w-full text-left px-3 py-1 text-xs text-[#cccccc] hover:bg-[#2a2d2e]">
          <Plus :size="12" class="text-[#89d185]" />
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
  </div>
</template>

<script setup>
import { ref, onMounted, computed, watch, nextTick, onUnmounted } from 'vue'
import { useConnectionStore } from '../stores/connection.js'
import {
  Plus, Server, Search, Upload, Download,
  Folder, FolderOpen, FolderPlus,
  List, LayoutGrid, Star,
  Zap, Pencil, Trash2,
} from 'lucide-vue-next'
import HostModal from './HostModal.vue'
import GroupModal from './GroupModal.vue'
import ConfirmDialog from './ConfirmDialog.vue'
import HostRow from './HostRow.vue'

const store = useConnectionStore()

const showModal = ref(false)
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
const draggingHost = ref(false)
const dragOverGroup = ref(null)
const customGroups = ref(new Set(JSON.parse(localStorage.getItem('host-custom-groups') || '[]')))

const contextMenu = ref({ show: false, x: 0, y: 0, type: '', data: null })
const pendingGroupForNewHost = ref(null)

const showGroupModal = ref(false)
const groupModalMode = ref('create')
const groupModalCurrentName = ref('')

const confirmDialog = ref({
  show: false,
  title: '',
  message: '',
  danger: false,
  onConfirm: () => {},
})

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

const displayHosts = computed(() => {
  if (viewMode.value === 'flat') return filteredHosts.value
  return filteredHosts.value.filter(h => !h.favorite)
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
  const nonFavorites = filteredHosts.value.filter(h => !h.favorite)
  for (const host of nonFavorites) {
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

const colorClassCache = new Map()

function groupColorClass(name) {
  if (colorClassCache.has(name)) {
    return colorClassCache.get(name)
  }
  const colors = [
    'hover:bg-blue-50 dark:hover:bg-blue-900/20',
    'hover:bg-green-50 dark:hover:bg-green-900/20',
    'hover:bg-purple-50 dark:hover:bg-purple-900/20',
    'hover:bg-orange-50 dark:hover:bg-orange-900/20',
    'hover:bg-pink-50 dark:hover:bg-pink-900/20',
    'hover:bg-cyan-50 dark:hover:bg-cyan-900/20',
    'hover:bg-yellow-50 dark:hover:bg-yellow-900/20',
    'hover:bg-red-50 dark:hover:bg-red-900/20',
  ]
  let hash = 0
  for (let i = 0; i < name.length; i++) {
    hash = ((hash << 5) - hash) + name.charCodeAt(i)
    hash |= 0
  }
  const result = colors[Math.abs(hash) % colors.length]
  colorClassCache.set(name, result)
  return result
}

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

function showGroupMenu(event, groupName) {
  event.preventDefault()
  event.stopPropagation()
  contextMenu.value = { show: true, x: event.clientX, y: event.clientY, type: 'group', data: groupName }
}

function showEmptyMenu(event) {
  contextMenu.value = { show: true, x: event.clientX, y: event.clientY, type: 'empty', data: null }
}

function showHostMenu(event, host) {
  event.stopPropagation()
  contextMenu.value = { show: true, x: event.clientX, y: event.clientY, type: 'host', data: host }
}

function hideContextMenu() {
  contextMenu.value.show = false
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
  draggingHost.value = false
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

function openConfirm(options) {
  confirmDialog.value = {
    show: true,
    title: options.title || 'Confirm',
    message: options.message || '',
    danger: options.danger || false,
    onConfirm: () => {
      confirmDialog.value.show = false
      options.onConfirm()
    },
  }
}

function onWindowClick() {
  hideContextMenu()
}

onMounted(() => {
  store.loadHosts()
  window.addEventListener('click', onWindowClick)
})

onUnmounted(() => {
  window.removeEventListener('click', onWindowClick)
})

function openModal() {
  editingHost.value = null
  showModal.value = true
}

function editHost(host) {
  editingHost.value = host
  showModal.value = true
}

async function handleSave({ id, hostData, password }) {
  if (id) {
    await store.updateHost(id, hostData)
    if (password) {
      await store.storePassword(id, password).catch((err) => {
        console.warn('Failed to store password in keyring:', err)
      })
    }
  } else {
    const newId = await store.addHost(hostData)
    if (password) {
      await store.storePassword(newId, password).catch((err) => {
        console.warn('Failed to store password in keyring:', err)
      })
    }
    if (pendingGroupForNewHost.value !== null) {
      await store.setHostGroup(newId, pendingGroupForNewHost.value)
      pendingGroupForNewHost.value = null
    }
  }
  showModal.value = false
  await store.loadHosts()
}

async function connectHost(id) {
  try {
    await store.connect(id)
  } catch (err) {
    console.error('Connection failed:', err)
  }
}

function deleteHost(host) {
  openConfirm({
    title: 'Delete Host',
    message: `Delete host "${host.name}" (${host.host})? This cannot be undone.`,
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

function importHosts() {
  importInput.value?.click()
}

async function onImportFileSelected(event) {
  const file = event.target.files[0]
  if (!file) return
  try {
    const text = await file.text()
    const count = await store.importHosts(text)
    window.dispatchEvent(new CustomEvent('app-toast', { detail: { message: `Imported ${count} hosts`, type: 'success' } }))
  } catch (err) {
    window.dispatchEvent(new CustomEvent('app-toast', { detail: { message: 'Import failed: ' + err, type: 'error' } }))
  }
  event.target.value = ''
}
</script>