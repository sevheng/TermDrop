<template>
  <div
    v-if="show"
    class="fixed inset-0 bg-black/60 flex items-center justify-center z-50"
    @click.self="onClose"
  >
    <div class="bg-[#252526] rounded-lg p-6 w-[28rem] border border-[#3c3c3c] shadow-xl">
      <h3 class="text-lg font-semibold text-[#cccccc] mb-5">
        {{ isEditing ? 'Edit Host' : 'Add Host' }}
      </h3>

      <div class="space-y-4">
        <!-- Name -->
        <div>
          <label class="block text-xs text-[#858585] mb-1.5">Name <span class="text-[#f44336]">*</span></label>
          <input
            v-model="form.name"
            type="text"
            placeholder="My Server"
            :class="inputClass('name')"
            @blur="validateField('name')"
          />
          <p v-if="errors.name" class="text-xs text-[#f44336] mt-1">{{ errors.name }}</p>
        </div>

        <!-- Host -->
        <div>
          <label class="block text-xs text-[#858585] mb-1.5">Host <span class="text-[#f44336]">*</span></label>
          <input
            v-model="form.host"
            type="text"
            placeholder="192.168.1.1 or example.com"
            :class="inputClass('host')"
            @blur="validateField('host')"
          />
          <p v-if="errors.host" class="text-xs text-[#f44336] mt-1">{{ errors.host }}</p>
        </div>

        <!-- Port + Username -->
        <div class="flex gap-3">
          <div class="flex-1">
            <label class="block text-xs text-[#858585] mb-1.5">Port <span class="text-[#f44336]">*</span></label>
            <input
              v-model.number="form.port"
              type="number"
              placeholder="22"
              :class="inputClass('port')"
              @blur="validateField('port')"
            />
            <p v-if="errors.port" class="text-xs text-[#f44336] mt-1">{{ errors.port }}</p>
          </div>
          <div class="flex-[2]">
            <label class="block text-xs text-[#858585] mb-1.5">Username <span class="text-[#f44336]">*</span></label>
            <input
              v-model="form.username"
              type="text"
              placeholder="root"
              :class="inputClass('username')"
              @blur="validateField('username')"
            />
            <p v-if="errors.username" class="text-xs text-[#f44336] mt-1">{{ errors.username }}</p>
          </div>
        </div>

        <!-- Auth Type Toggle -->
        <div>
          <label class="block text-xs text-[#858585] mb-1.5">Authentication</label>
          <div class="flex bg-[#3c3c3c] rounded p-1">
            <button
              type="button"
              @click="form.auth_type = 'password'"
              class="flex-1 py-1.5 text-sm rounded transition-colors"
              :class="form.auth_type === 'password' ? 'bg-[#0e639c] text-white' : 'text-[#858585] hover:text-[#cccccc]'"
            >
              Password
            </button>
            <button
              type="button"
              @click="form.auth_type = 'key'"
              class="flex-1 py-1.5 text-sm rounded transition-colors"
              :class="form.auth_type === 'key' ? 'bg-[#0e639c] text-white' : 'text-[#858585] hover:text-[#cccccc]'"
            >
              SSH Key
            </button>
          </div>
        </div>

        <!-- Password -->
        <div v-if="form.auth_type === 'password'">
          <label class="block text-xs text-[#858585] mb-1.5">
            Password
            <span v-if="!isEditing" class="text-[#f44336]">*</span>
          </label>
          <div class="relative">
            <input
              v-model="form.password"
              :type="showPassword ? 'text' : 'password'"
              :placeholder="isEditing ? 'Leave empty to keep existing' : ''"
              :class="inputClass('password')"
              @blur="validateField('password')"
            />
            <button
              type="button"
              @click="showPassword = !showPassword"
              class="absolute right-2.5 top-1/2 -translate-y-1/2 text-[#6e6e6e] hover:text-[#cccccc]"
            >
              <component :is="showPassword ? EyeOff : Eye" :size="16" />
            </button>
          </div>
          <p v-if="errors.password" class="text-xs text-[#f44336] mt-1">{{ errors.password }}</p>
          <p v-else-if="isEditing" class="text-xs text-[#6e6e6e] mt-1">Leave empty to keep the existing password</p>
        </div>

        <!-- Key Path -->
        <div v-if="form.auth_type === 'key'">
          <label class="block text-xs text-[#858585] mb-1.5">Private Key Path <span class="text-[#f44336]">*</span></label>
          <div class="flex gap-2">
            <input
              v-model="form.key_path"
              type="text"
              placeholder="~/.ssh/id_rsa"
              :class="[inputClass('key_path'), 'flex-1']"
              @blur="validateField('key_path')"
            />
            <button
              type="button"
              @click="browseKey"
              class="px-3 py-2 bg-[#3c3c3c] border border-[#3c3c3c] rounded text-sm text-[#cccccc] hover:bg-[#37373d] shrink-0"
            >
              <FileSearch :size="14" class="inline mr-1" />
              Browse
            </button>
          </div>
          <p v-if="errors.key_path" class="text-xs text-[#f44336] mt-1">{{ errors.key_path }}</p>
          <p v-else class="text-xs text-[#6e6e6e] mt-1">Supports ~ for home directory</p>
        </div>
      </div>

      <!-- Actions -->
      <div class="flex justify-end gap-2 mt-6">
        <button
          @click="onClose"
          :disabled="loading"
          class="px-4 py-2 text-sm text-[#858585] hover:text-[#cccccc] disabled:opacity-50"
        >
          Cancel
        </button>
        <button
          @click="onSave"
          :disabled="loading"
          class="px-4 py-2 text-sm bg-[#0e639c] hover:bg-[#1177bb] disabled:bg-[#0e639c]/50 disabled:opacity-70 text-white rounded flex items-center gap-2"
        >
          <Loader2 v-if="loading" :size="14" class="animate-spin" />
          {{ loading ? 'Saving...' : (isEditing ? 'Save Changes' : 'Add Host') }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, watch, computed } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { Eye, EyeOff, Loader2, FileSearch } from 'lucide-vue-next'

const props = defineProps({
  show: Boolean,
  host: Object, // null = add mode, object = edit mode
})

const emit = defineEmits(['save', 'close'])

const isEditing = computed(() => !!props.host)

const form = ref({
  name: '',
  host: '',
  port: 22,
  username: '',
  auth_type: 'password',
  key_path: '',
  password: '',
})

const errors = ref({})
const loading = ref(false)
const showPassword = ref(false)

function resetForm() {
  if (props.host) {
    form.value = {
      name: props.host.name || '',
      host: props.host.host || '',
      port: props.host.port || 22,
      username: props.host.username || '',
      auth_type: props.host.auth_type || 'password',
      key_path: props.host.key_path || '',
      password: '',
    }
  } else {
    form.value = {
      name: '',
      host: '',
      port: 22,
      username: '',
      auth_type: 'password',
      key_path: '',
      password: '',
    }
  }
  errors.value = {}
  showPassword.value = false
}

watch(() => props.show, (visible) => {
  if (visible) resetForm()
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
    case 'host':
      if (!val || String(val).trim() === '') msg = 'Host is required'
      break
    case 'port':
      if (val === '' || val === null || val === undefined) msg = 'Port is required'
      else if (!Number.isInteger(Number(val))) msg = 'Port must be an integer'
      else if (Number(val) < 1 || Number(val) > 65535) msg = 'Port must be 1–65535'
      break
    case 'username':
      if (!val || String(val).trim() === '') msg = 'Username is required'
      break
    case 'password':
      if (form.value.auth_type === 'password' && !isEditing.value) {
        if (!val || String(val).trim() === '') msg = 'Password is required for new hosts'
      }
      break
    case 'key_path':
      if (form.value.auth_type === 'key') {
        if (!val || String(val).trim() === '') msg = 'Key path is required'
      }
      break
  }

  if (msg) errors.value[field] = msg
  else delete errors.value[field]
}

function validateAll() {
  ;['name', 'host', 'port', 'username'].forEach(validateField)
  if (form.value.auth_type === 'password') validateField('password')
  if (form.value.auth_type === 'key') validateField('key_path')
  return Object.keys(errors.value).length === 0
}

async function browseKey() {
  const selected = await open({ multiple: false, directory: false })
  if (selected) {
    form.value.key_path = Array.isArray(selected) ? selected[0] : selected
    validateField('key_path')
  }
}

function onClose() {
  if (loading.value) return
  emit('close')
}

async function onSave() {
  if (!validateAll()) return
  loading.value = true
  try {
    const hostData = {
      name: form.value.name.trim(),
      host: form.value.host.trim(),
      port: Number(form.value.port),
      username: form.value.username.trim(),
      auth_type: form.value.auth_type,
      key_path: form.value.auth_type === 'key' ? form.value.key_path.trim() : null,
    }

    const password = form.value.auth_type === 'password' ? form.value.password : null

    emit('save', {
      id: props.host?.id ?? null,
      hostData,
      password,
    })
  } finally {
    loading.value = false
  }
}

// Escape key closes modal
function onKeydown(e) {
  if (e.key === 'Escape' && props.show) onClose()
}

watch(() => props.show, (visible) => {
  if (visible) {
    window.addEventListener('keydown', onKeydown)
  } else {
    window.removeEventListener('keydown', onKeydown)
  }
})
</script>
