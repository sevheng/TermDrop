<template>
  <div class="h-full w-full flex flex-col">
    <div class="p-4 border-b border-gray-700 flex items-center justify-between">
      <h2 class="text-sm font-semibold text-gray-200">Hosts</h2>
      <button @click="openModal()" class="text-gray-400 hover:text-white">
        <Plus :size="16" />
      </button>
    </div>

    <!-- Search -->
    <div class="px-3 py-2 border-b border-gray-700">
      <div class="relative">
        <Search :size="14" class="absolute left-2 top-1/2 -translate-y-1/2 text-gray-500" />
        <input
          v-model="searchQuery"
          type="text"
          placeholder="Search hosts..."
          class="w-full bg-gray-700 border border-gray-600 rounded pl-7 pr-2 py-1.5 text-sm text-white placeholder-gray-500 focus:outline-none focus:border-blue-500"
        />
      </div>
    </div>

    <div class="flex-1 overflow-y-auto p-2">
      <!-- Empty state -->
      <div v-if="filteredHosts.length === 0" class="flex flex-col items-center justify-center py-8 text-gray-500">
        <Server :size="32" class="mb-2 opacity-50" />
        <p class="text-sm">
          {{ store.hosts.length === 0 ? 'No hosts yet' : 'No matching hosts' }}
        </p>
        <p v-if="store.hosts.length === 0" class="text-xs mt-1">
          Click + to add your first host
        </p>
      </div>

      <div
        v-for="host in filteredHosts"
        :key="host.id"
        class="group flex items-center justify-between p-2 rounded hover:bg-gray-700 cursor-pointer"
        @click="connectHost(host.id)"
      >
        <div class="flex items-center gap-2 min-w-0">
          <Server :size="14" class="text-gray-400 shrink-0" />
          <div class="min-w-0">
            <div class="text-sm text-gray-200 truncate">{{ host.name }}</div>
            <div class="text-xs text-gray-500 truncate">{{ host.username }}@{{ host.host }}:{{ host.port }}</div>
          </div>
        </div>
        <div class="flex items-center gap-1 opacity-0 group-hover:opacity-100 shrink-0">
          <button @click.stop="editHost(host)" class="text-gray-400 hover:text-white p-1">
            <Pencil :size="12" />
          </button>
          <button @click.stop="deleteHost(host)" class="text-gray-400 hover:text-red-400 p-1">
            <Trash2 :size="12" />
          </button>
        </div>
      </div>
    </div>

    <HostModal
      :show="showModal"
      :host="editingHost"
      @close="showModal = false"
      @save="handleSave"
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
  </div>
</template>

<script setup>
import { ref, onMounted, computed } from 'vue'
import { useConnectionStore } from '../stores/connection.js'
import { Plus, Server, Pencil, Trash2, Search } from 'lucide-vue-next'
import HostModal from './HostModal.vue'
import ConfirmDialog from './ConfirmDialog.vue'

const store = useConnectionStore()

const showModal = ref(false)
const editingHost = ref(null)
const searchQuery = ref('')
const confirmDialog = ref({
  show: false,
  title: '',
  message: '',
  danger: false,
  onConfirm: () => {},
})

const filteredHosts = computed(() => {
  const q = searchQuery.value.trim().toLowerCase()
  if (!q) return store.hosts
  return store.hosts.filter(h =>
    h.name.toLowerCase().includes(q) ||
    h.host.toLowerCase().includes(q) ||
    h.username.toLowerCase().includes(q)
  )
})

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

onMounted(() => {
  store.loadHosts()
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
</script>
