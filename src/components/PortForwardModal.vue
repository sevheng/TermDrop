<template>
  <div
    v-if="show"
    class="fixed inset-0 bg-black/60 flex items-center justify-center z-50"
    @click.self="onClose"
  >
    <div class="bg-white rounded-lg p-6 w-[28rem] border border-gray-200 shadow-xl dark:bg-gray-800 dark:border-gray-700">
      <h3 class="text-lg font-semibold text-gray-900 mb-5 dark:text-white">Add Port Forward</h3>

      <div class="space-y-4">
        <!-- Name -->
        <div>
          <label class="block text-xs text-gray-500 mb-1.5 dark:text-gray-400">Name <span class="text-red-500">*</span></label>
          <input
            v-model="form.name"
            type="text"
            placeholder="MySQL Tunnel"
            :class="inputClass('name')"
            @blur="validateField('name')"
          />
          <p v-if="errors.name" class="text-xs text-red-500 mt-1">{{ errors.name }}</p>
        </div>

        <!-- Kind -->
        <div>
          <label class="block text-xs text-gray-500 mb-1.5 dark:text-gray-400">Type</label>
          <div class="flex bg-gray-100 rounded p-1 dark:bg-gray-700">
            <button
              type="button"
              @click="form.kind = 'local'"
              class="flex-1 py-1.5 text-sm rounded transition-colors"
              :class="form.kind === 'local' ? 'bg-blue-600 text-white' : 'text-gray-600 hover:text-gray-900 dark:text-gray-300 dark:hover:text-white'"
            >
              Local
            </button>
            <button
              type="button"
              @click="form.kind = 'dynamic'"
              class="flex-1 py-1.5 text-sm rounded transition-colors"
              :class="form.kind === 'dynamic' ? 'bg-blue-600 text-white' : 'text-gray-600 hover:text-gray-900 dark:text-gray-300 dark:hover:text-white'"
            >
              Dynamic (SOCKS)
            </button>
          </div>
        </div>

        <!-- Local Address -->
        <div class="flex gap-3">
          <div class="flex-[2]">
            <label class="block text-xs text-gray-500 mb-1.5 dark:text-gray-400">Local Host</label>
            <input v-model="form.local_host" type="text" :class="inputClass('local_host')" />
          </div>
          <div class="flex-1">
            <label class="block text-xs text-gray-500 mb-1.5 dark:text-gray-400">Local Port <span class="text-red-500">*</span></label>
            <input
              v-model.number="form.local_port"
              type="number"
              placeholder="13306"
              :class="inputClass('local_port')"
              @blur="validateField('local_port')"
            />
            <p v-if="errors.local_port" class="text-xs text-red-500 mt-1">{{ errors.local_port }}</p>
          </div>
        </div>

        <!-- Remote Address (local only) -->
        <div v-if="form.kind === 'local'" class="flex gap-3">
          <div class="flex-[2]">
            <label class="block text-xs text-gray-500 mb-1.5 dark:text-gray-400">Remote Host <span class="text-red-500">*</span></label>
            <input
              v-model="form.remote_host"
              type="text"
              placeholder="localhost"
              :class="inputClass('remote_host')"
              @blur="validateField('remote_host')"
            />
            <p v-if="errors.remote_host" class="text-xs text-red-500 mt-1">{{ errors.remote_host }}</p>
          </div>
          <div class="flex-1">
            <label class="block text-xs text-gray-500 mb-1.5 dark:text-gray-400">Remote Port <span class="text-red-500">*</span></label>
            <input
              v-model.number="form.remote_port"
              type="number"
              placeholder="3306"
              :class="inputClass('remote_port')"
              @blur="validateField('remote_port')"
            />
            <p v-if="errors.remote_port" class="text-xs text-red-500 mt-1">{{ errors.remote_port }}</p>
          </div>
        </div>

        <!-- Dynamic hint -->
        <p v-else class="text-xs text-gray-500 dark:text-gray-400">
          Dynamic forwarding creates a SOCKS5 proxy on the local address. Configure your browser or app to use <code class="bg-gray-100 px-1 rounded dark:bg-gray-700">{{ form.local_host }}:{{ form.local_port }}</code> as a SOCKS5 proxy.
        </p>
      </div>

      <div class="flex justify-end gap-2 mt-6">
        <button @click="onClose" class="px-4 py-2 text-sm text-gray-600 hover:text-gray-900 dark:text-gray-300 dark:hover:text-white">Cancel</button>
        <button
          @click="onSave"
          :disabled="loading"
          class="px-4 py-2 text-sm bg-blue-600 hover:bg-blue-700 disabled:bg-blue-400 disabled:opacity-70 text-white rounded flex items-center gap-2"
        >
          <Loader2 v-if="loading" :size="14" class="animate-spin" />
          {{ loading ? 'Adding...' : 'Add Forward' }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, watch } from 'vue'
import { Loader2 } from 'lucide-vue-next'

const props = defineProps({
  show: Boolean,
  hostId: Number,
})

const emit = defineEmits(['save', 'close'])

const form = ref({
  name: '',
  kind: 'local',
  local_host: '127.0.0.1',
  local_port: null,
  remote_host: 'localhost',
  remote_port: null,
})

const errors = ref({})
const loading = ref(false)

watch(() => props.show, (visible) => {
  if (visible) {
    form.value = {
      name: '',
      kind: 'local',
      local_host: '127.0.0.1',
      local_port: null,
      remote_host: 'localhost',
      remote_port: null,
    }
    errors.value = {}
  }
})

function inputClass(field) {
  const base = 'w-full bg-gray-100 border rounded px-3 py-2 text-sm text-gray-900 focus:outline-none transition-colors dark:bg-gray-700 dark:text-white'
  const error = errors.value[field] ? 'border-red-500 focus:border-red-400' : 'border-gray-300 focus:border-blue-500 dark:border-gray-600'
  return `${base} ${error}`
}

function validateField(field) {
  const val = form.value[field]
  let msg = ''

  switch (field) {
    case 'name':
      if (!val || String(val).trim() === '') msg = 'Name is required'
      break
    case 'local_port':
      if (!val || val === '') msg = 'Local port is required'
      else if (val < 1 || val > 65535) msg = 'Port must be 1–65535'
      break
    case 'remote_host':
      if (form.value.kind === 'local' && (!val || String(val).trim() === '')) msg = 'Remote host is required'
      break
    case 'remote_port':
      if (form.value.kind === 'local' && (!val || val === '')) msg = 'Remote port is required'
      else if (form.value.kind === 'local' && (val < 1 || val > 65535)) msg = 'Port must be 1–65535'
      break
  }

  if (msg) errors.value[field] = msg
  else delete errors.value[field]
}

function validateAll() {
  ;['name', 'local_port'].forEach(validateField)
  if (form.value.kind === 'local') {
    ;['remote_host', 'remote_port'].forEach(validateField)
  }
  return Object.keys(errors.value).length === 0
}

function onClose() {
  emit('close')
}

async function onSave() {
  if (!validateAll()) return
  loading.value = true
  try {
    const forwardData = {
      host_id: props.hostId,
      name: form.value.name.trim(),
      kind: form.value.kind,
      local_host: form.value.local_host.trim() || '127.0.0.1',
      local_port: Number(form.value.local_port),
      remote_host: form.value.kind === 'local' ? form.value.remote_host.trim() : null,
      remote_port: form.value.kind === 'local' ? Number(form.value.remote_port) : null,
    }
    emit('save', forwardData)
  } finally {
    loading.value = false
  }
}
</script>
