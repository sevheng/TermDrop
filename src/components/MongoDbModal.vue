<template>
  <ModalShell :show="show" dim="bg-black/60" z="z-50" panel-class="p-6 w-[28rem] shadow-xl max-h-[90vh] overflow-y-auto">
      <h3 class="text-lg font-semibold text-[#cccccc] mb-5">
        {{ isEditing ? 'Edit MongoDB connection' : 'Add MongoDB connection' }}
      </h3>

      <div class="space-y-4">
        <!-- Name -->
        <div>
          <label class="block text-xs text-[#858585] mb-1.5">
            Name <span class="text-[#f44336]">*</span>
          </label>
          <input
            ref="nameInput"
            v-model="form.name"
            type="text"
            placeholder="Staging DB"
            :class="inputClass('name')"
            @blur="validateField('name')"
            @keydown.enter="onSave"
          />
          <p v-if="errors.name" class="text-xs text-[#f44336] mt-1">{{ errors.name }}</p>
        </div>

        <!-- Connection -->
        <div class="border border-[#3c3c3c] rounded-lg p-4 space-y-3">
          <h4 class="text-xs font-semibold text-[#cccccc] uppercase tracking-wider">Connection</h4>
          <MongoConnectionFields
            v-model="fields"
            :errors="fieldErrors"
            :is-srv="isSrv"
            :has-stored-secret="hasStoredSecret"
            host-placeholder="host or IP"
            options-placeholder="retryWrites=true&replicaSet=rs0"
            @uri-input="syncFormFromUri"
            @field-input="rebuildUri"
            @validate="validateField($event)"
            @save="onSave"
          />
        </div>
      </div>

      <div class="flex justify-end gap-2 mt-6">
        <button
          @click="onClose"
          class="px-4 py-2 text-sm text-[#858585] hover:text-[#cccccc] rounded hover:bg-[#2a2d2e] transition-colors"
        >
          Cancel
        </button>
        <button
          @click="onSave"
          class="px-4 py-2 text-sm text-white rounded bg-[#0e639c] hover:bg-[#1177bb] transition-colors"
        >
          {{ isEditing ? 'Save' : 'Add' }}
        </button>
      </div>
  </ModalShell>
</template>

<script setup>
import { ref, computed, watch, nextTick, onUnmounted } from 'vue'
import ModalShell from './ModalShell.vue'
import MongoConnectionFields from './MongoConnectionFields.vue'
import { parseMongoUri, buildMongoUri, parseUriToForm } from '../composables/useMongoUri.js'
import { invoke } from '../utils/invoke.js'

const props = defineProps({
  show: Boolean,
  host: Object,
})

const emit = defineEmits(['save', 'close'])

const isEditing = computed(() => !!props.host)

const nameInput = ref(null)
const loading = ref(false)

const defaultForm = () => ({
  name: '',
  uri: '',
  host: '',
  port: 27017,
  username: '',
  password: '',
  database: '',
  authSource: 'admin',
  options: '',
})

const form = ref(defaultForm())
const errors = ref({})

/** SRV seedlist or multi-host URIs have no single host/port to require. */
function isSrvLikeUri(uri) {
  const trimmed = (uri || '').trim()
  return trimmed.startsWith('mongodb+srv://') || /^mongodb:\/\/[^/]*,/.test(trimmed)
}
const isSrv = computed(() => isSrvLikeUri(form.value.uri))

const FIELD_KEYS = ['uri', 'host', 'port', 'username', 'password', 'database', 'authSource', 'options']

/** The connection fields as one object, for MongoConnectionFields' v-model. */
const fields = computed({
  get: () => Object.fromEntries(FIELD_KEYS.map(key => [key, form.value[key]])),
  set: (value) => {
    for (const key of FIELD_KEYS) form.value[key] = value[key]
  },
})

const fieldErrors = computed(() =>
  Object.fromEntries(FIELD_KEYS.map(key => [key, errors.value[key]])),
)

function resetForm() {
  errors.value = {}

  if (props.host) {
    const parsed = parseMongoUri(props.host.mongo_uri)
    const uri = parsed.mode === 'form'
      ? buildMongoUri({
          scheme: 'mongodb',
          host: parsed.host,
          port: parsed.port,
          username: parsed.username,
          password: parsed.password,
          database: parsed.database,
          authSource: parsed.authSource,
          options: parsed.options,
        })
      : parsed.uri || ''

    form.value = {
      name: props.host.name || '',
      uri,
      host: parsed.mode === 'form' ? parsed.host : '',
      port: parsed.mode === 'form' ? parsed.port : 27017,
      username: parsed.mode === 'form' ? parsed.username : '',
      password: parsed.mode === 'form' ? parsed.password : '',
      database: parsed.mode === 'form' ? parsed.database : '',
      authSource: parsed.mode === 'form' ? parsed.authSource : 'admin',
      options: parsed.mode === 'form' ? parsed.options : '',
    }
  } else {
    form.value = defaultForm()
  }

  nextTick(() => nameInput.value?.focus())
}

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
    case 'uri':
      if (!val || String(val).trim() === '') {
        msg = 'Connection string is required'
      } else if (!/^mongodb(\+srv)?:\/\//.test(String(val).trim())) {
        msg = 'URI must start with mongodb:// or mongodb+srv://'
      }
      break
    case 'host':
      if (!isSrv.value && (!val || String(val).trim() === '')) msg = 'Host is required'
      break
    case 'port':
      if (!isSrv.value) {
        if (val === '' || val === null || val === undefined) msg = 'Port is required'
        else if (!Number.isInteger(Number(val))) msg = 'Port must be an integer'
        else if (Number(val) < 1 || Number(val) > 65535) msg = 'Port must be 1–65535'
      }
      break
    // Database is optional; the app lists every database when none is given.
  }

  if (msg) errors.value[field] = msg
  else delete errors.value[field]
}

function validateAll() {
  ;['name', 'uri', 'host', 'port'].forEach(validateField)
  return Object.keys(errors.value).length === 0
}

// While the fields are being filled from the URI, ignore rebuild requests so
// the URI the user typed is not rewritten under them.
let syncingFromUri = false

/** Fill the structured fields from the connection string. */
function syncFormFromUri() {
  const parsed = parseUriToForm(form.value.uri)
  if (!parsed) return

  syncingFromUri = true
  form.value.host = parsed.host
  form.value.port = parsed.port
  form.value.username = parsed.username
  form.value.password = parsed.password
  form.value.database = parsed.database
  form.value.authSource = parsed.authSource
  form.value.options = parsed.options
  nextTick(() => {
    syncingFromUri = false
  })
}

/** Rebuild the connection string from the structured fields. */
function rebuildUri() {
  if (syncingFromUri) return
  form.value.uri = buildMongoUri({
    scheme: 'mongodb',
    host: form.value.host,
    port: form.value.port,
    username: form.value.username,
    password: form.value.password,
    database: form.value.database,
    authSource: form.value.authSource,
    options: form.value.options,
  })
}

async function onSave() {
  if (!validateAll()) return

  loading.value = true
  try {
    emit('save', {
      id: props.host?.id ?? null,
      name: form.value.name.trim(),
      mongo_uri: form.value.uri.trim(),
    })
  } finally {
    loading.value = false
  }
}

function onClose() {
  if (loading.value) return
  emit('close')
}

function onKeydown(e) {
  if (e.key === 'Escape' && props.show) onClose()
}

watch(() => props.show, (visible) => {
  if (visible) {
    resetForm()
    loadStoredSecret()
    window.addEventListener('keydown', onKeydown)
  } else {
    window.removeEventListener('keydown', onKeydown)
  }
})

/**
 * Whether a password is already in the keyring.
 *
 * The password itself is never sent to the frontend, so the field renders empty
 * on edit; without this the user cannot tell "no password" from "not shown".
 */
const hasStoredSecret = ref(false)

async function loadStoredSecret() {
  hasStoredSecret.value = false
  const hostId = props.host?.id
  if (!hostId) return
  try {
    hasStoredSecret.value = await invoke('mongodb_has_secret', { hostId })
  } catch {
    // Not knowing is not worth blocking the dialog over.
    hasStoredSecret.value = false
  }
}

onUnmounted(() => {
  window.removeEventListener('keydown', onKeydown)
})
</script>
