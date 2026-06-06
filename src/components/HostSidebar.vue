<template>
  <div class="w-64 h-full bg-gray-800 border-r border-gray-700 flex flex-col">
    <div class="p-4 border-b border-gray-700 flex items-center justify-between">
      <h2 class="text-sm font-semibold text-gray-200">Hosts</h2>
      <button @click="openModal()" class="text-gray-400 hover:text-white">
        <Plus :size="16" />
      </button>
    </div>

    <div class="flex-1 overflow-y-auto p-2">
      <div
        v-for="host in store.hosts"
        :key="host.id"
        class="group flex items-center justify-between p-2 rounded hover:bg-gray-700 cursor-pointer"
        @click="store.connect(host.id)"
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
          <button @click.stop="deleteHost(host.id)" class="text-gray-400 hover:text-red-400 p-1">
            <Trash2 :size="12" />
          </button>
        </div>
      </div>
    </div>

    <!-- Add/Edit Modal -->
    <div v-if="showModal" class="absolute inset-0 bg-black/50 flex items-center justify-center z-50">
      <div class="bg-gray-800 rounded-lg p-6 w-96 border border-gray-700">
        <h3 class="text-lg font-semibold text-white mb-4">{{ editingId ? 'Edit Host' : 'Add Host' }}</h3>
        
        <div class="space-y-3">
          <div>
            <label class="block text-xs text-gray-400 mb-1">Name</label>
            <input v-model="form.name" type="text" class="w-full bg-gray-700 border border-gray-600 rounded px-3 py-2 text-sm text-white focus:outline-none focus:border-blue-500" />
          </div>
          <div>
            <label class="block text-xs text-gray-400 mb-1">Host</label>
            <input v-model="form.host" type="text" class="w-full bg-gray-700 border border-gray-600 rounded px-3 py-2 text-sm text-white focus:outline-none focus:border-blue-500" />
          </div>
          <div class="flex gap-3">
            <div class="flex-1">
              <label class="block text-xs text-gray-400 mb-1">Port</label>
              <input v-model.number="form.port" type="number" class="w-full bg-gray-700 border border-gray-600 rounded px-3 py-2 text-sm text-white focus:outline-none focus:border-blue-500" />
            </div>
            <div class="flex-1">
              <label class="block text-xs text-gray-400 mb-1">Username</label>
              <input v-model="form.username" type="text" class="w-full bg-gray-700 border border-gray-600 rounded px-3 py-2 text-sm text-white focus:outline-none focus:border-blue-500" />
            </div>
          </div>
          <div>
            <label class="block text-xs text-gray-400 mb-1">Auth Type</label>
            <select v-model="form.auth_type" class="w-full bg-gray-700 border border-gray-600 rounded px-3 py-2 text-sm text-white focus:outline-none focus:border-blue-500">
              <option value="password">Password</option>
              <option value="key">SSH Key</option>
            </select>
          </div>
          <div v-if="form.auth_type === 'password'">
            <label class="block text-xs text-gray-400 mb-1">Password</label>
            <input v-model="form.password" type="password" class="w-full bg-gray-700 border border-gray-600 rounded px-3 py-2 text-sm text-white focus:outline-none focus:border-blue-500" />
          </div>
          <div v-if="form.auth_type === 'key'">
            <label class="block text-xs text-gray-400 mb-1">Key Path</label>
            <input v-model="form.key_path" type="text" class="w-full bg-gray-700 border border-gray-600 rounded px-3 py-2 text-sm text-white focus:outline-none focus:border-blue-500" placeholder="~/.ssh/id_rsa" />
          </div>
        </div>

        <div class="flex justify-end gap-2 mt-6">
          <button @click="showModal = false" class="px-4 py-2 text-sm text-gray-300 hover:text-white">Cancel</button>
          <button @click="saveHost" class="px-4 py-2 text-sm bg-blue-600 hover:bg-blue-700 text-white rounded">Save</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useConnectionStore } from '../stores/connection.js'
import { Plus, Server, Pencil, Trash2 } from 'lucide-vue-next'

const store = useConnectionStore()

const showModal = ref(false)
const editingId = ref(null)
const form = ref({
  name: '',
  host: '',
  port: 22,
  username: '',
  auth_type: 'password',
  key_path: '',
  password: '',
})

onMounted(() => {
  store.loadHosts()
})

function openModal() {
  editingId.value = null
  form.value = {
    name: '',
    host: '',
    port: 22,
    username: '',
    auth_type: 'password',
    key_path: '',
    password: '',
  }
  showModal.value = true
}

function editHost(host) {
  editingId.value = host.id
  form.value = {
    name: host.name,
    host: host.host,
    port: host.port,
    username: host.username,
    auth_type: host.auth_type,
    key_path: host.key_path || '',
    password: '',
  }
  showModal.value = true
}

async function saveHost() {
  const hostData = {
    name: form.value.name,
    host: form.value.host,
    port: form.value.port,
    username: form.value.username,
    auth_type: form.value.auth_type,
    key_path: form.value.key_path || null,
  }

  if (editingId.value) {
    await store.updateHost(editingId.value, hostData)
  } else {
    const id = await store.addHost(hostData)
    if (form.value.password && form.value.auth_type === 'password') {
      await store.storePassword(id, form.value.password)
    }
  }
  showModal.value = false
  await store.loadHosts()
}

async function deleteHost(id) {
  if (confirm('Delete this host?')) {
    await store.removeHost(id)
  }
}
</script>
