<template>
  <div
    v-if="show"
    class="fixed inset-0 bg-black/60 flex items-center justify-center z-50"
    @click.self="onClose"
  >
    <div class="bg-[#252526] rounded-lg p-6 w-[28rem] border border-[#3c3c3c] shadow-xl">
      <h3 class="text-lg font-semibold text-[#cccccc] mb-5">{{ isEditing ? 'Edit Port Forward' : 'Add Port Forward' }}</h3>

      <div class="space-y-4">
        <!-- Name -->
        <div>
          <label class="block text-xs text-[#858585] mb-1.5">Name <span class="text-[#f44336]">*</span></label>
          <input
            v-model="form.name"
            type="text"
            placeholder="MySQL Tunnel"
            :class="inputClass('name')"
            @blur="validateField('name')"
          />
          <p v-if="errors.name" class="text-xs text-[#f44336] mt-1">{{ errors.name }}</p>
        </div>

        <!-- Kind -->
        <div>
          <label class="block text-xs text-[#858585] mb-1.5">Type</label>
          <div class="flex bg-[#3c3c3c] rounded p-1">
            <button
              type="button"
              @click="form.kind = 'local'"
              class="flex-1 py-1.5 text-sm rounded transition-colors"
              :class="form.kind === 'local' ? 'bg-[#0e639c] text-white' : 'text-[#858585] hover:text-[#cccccc]'"
            >
              Local
            </button>
            <button
              type="button"
              @click="form.kind = 'dynamic'"
              class="flex-1 py-1.5 text-sm rounded transition-colors"
              :class="form.kind === 'dynamic' ? 'bg-[#0e639c] text-white' : 'text-[#858585] hover:text-[#cccccc]'"
            >
              Dynamic (SOCKS)
            </button>
          </div>
        </div>

        <!-- Local Address -->
        <div class="flex gap-3">
          <div class="flex-[2]">
            <label class="block text-xs text-[#858585] mb-1.5">Local Host</label>
            <input v-model="form.local_host" type="text" :class="inputClass('local_host')" />
          </div>
          <div class="flex-1">
            <label class="block text-xs text-[#858585] mb-1.5">Local Port <span class="text-[#f44336]">*</span></label>
            <input
              v-model.number="form.local_port"
              type="number"
              placeholder="13306"
              :class="inputClass('local_port')"
              @blur="validateField('local_port')"
            />
            <p v-if="errors.local_port" class="text-xs text-[#f44336] mt-1">{{ errors.local_port }}</p>
          </div>
        </div>

        <!-- Remote Address (local only) -->
        <div v-if="form.kind === 'local'" class="flex gap-3">
          <div class="flex-[2]">
            <label class="block text-xs text-[#858585] mb-1.5">Remote Host <span class="text-[#f44336]">*</span></label>
            <input
              v-model="form.remote_host"
              type="text"
              placeholder="localhost"
              :class="inputClass('remote_host')"
              @blur="validateField('remote_host')"
            />
            <p v-if="errors.remote_host" class="text-xs text-[#f44336] mt-1">{{ errors.remote_host }}</p>
          </div>
          <div class="flex-1">
            <label class="block text-xs text-[#858585] mb-1.5">Remote Port <span class="text-[#f44336]">*</span></label>
            <input
              v-model.number="form.remote_port"
              type="number"
              placeholder="3306"
              :class="inputClass('remote_port')"
              @blur="validateField('remote_port')"
            />
            <p v-if="errors.remote_port" class="text-xs text-[#f44336] mt-1">{{ errors.remote_port }}</p>
          </div>
        </div>

        <!-- Dynamic hint -->
        <p v-else class="text-xs text-[#858585]">
          Dynamic forwarding creates a SOCKS5 proxy on the local address. Configure your browser or app to use <code class="bg-[#3c3c3c] px-1 rounded">{{ form.local_host }}:{{ form.local_port }}</code> as a SOCKS5 proxy.
        </p>
      </div>

      <div class="flex justify-end gap-2 mt-6">
        <button @click="onClose" class="px-4 py-2 text-sm text-[#858585] hover:text-[#cccccc]">Cancel</button>
        <button
          @click="onSave"
          :disabled="loading"
          class="px-4 py-2 text-sm bg-[#0e639c] hover:bg-[#1177bb] disabled:bg-[#0e639c]/50 disabled:opacity-70 text-white rounded flex items-center gap-2"
        >
          <Loader2 v-if="loading" :size="14" class="animate-spin" />
          {{ loading ? 'Saving...' : (isEditing ? 'Save Changes' : 'Add Forward') }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, watch, computed } from 'vue'
import { Loader2 } from 'lucide-vue-next'

const props = defineProps({
  show: Boolean,
  hostId: Number,
  prefill: Object,
  editForward: Object,
})

const emit = defineEmits(['save', 'close'])

const isEditing = computed(() => !!props.editForward)

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
    if (props.editForward) {
      form.value = {
        name: props.editForward.name || '',
        kind: props.editForward.kind || 'local',
        local_host: props.editForward.local_host || '127.0.0.1',
        local_port: props.editForward.local_port ?? null,
        remote_host: props.editForward.remote_host || 'localhost',
        remote_port: props.editForward.remote_port ?? null,
      }
    } else if (props.prefill) {
      form.value = {
        name: props.prefill.name || '',
        kind: props.prefill.kind || 'local',
        local_host: props.prefill.local_host || '127.0.0.1',
        local_port: props.prefill.local_port ?? null,
        remote_host: props.prefill.remote_host || 'localhost',
        remote_port: props.prefill.remote_port ?? null,
      }
    } else {
      form.value = {
        name: '',
        kind: 'local',
        local_host: '127.0.0.1',
        local_port: null,
        remote_host: 'localhost',
        remote_port: null,
      }
    }
    errors.value = {}
  }
})

function inputClass(field) {
  const base = 'w-full bg-[#3c3c3c] border rounded px-3 py-2 text-sm text-[#cccccc] focus:outline-none transition-colors'
  const error = errors.value[field] ? 'border-[#f44336] focus:border-[#f44336]' : 'border-[#3c3c3c] focus:border-[#007acc]'
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
    emit('save', {
      id: props.editForward?.id ?? null,
      forwardData,
    })
  } finally {
    loading.value = false
  }
}
</script>
