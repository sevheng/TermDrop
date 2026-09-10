<template>
  <ModalShell :show="show" dim="bg-black/60" z="z-50" panel-class="p-6 w-[28rem] shadow-xl max-h-[90vh] overflow-y-auto">
      <h3 class="text-lg font-semibold text-[#cccccc] mb-5">
        {{ isEditing ? 'Edit MongoDB' : 'Add MongoDB' }}
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

        <!-- Remote Connection -->
        <div class="border border-[#3c3c3c] rounded-lg p-4 space-y-3">
          <h4 class="text-xs font-semibold text-[#cccccc] uppercase tracking-wider">Remote Connection</h4>
          <MongoConnectionFields
            v-model="remoteFields"
            :errors="remoteErrors"
            :is-srv="isRemoteSrv"
            :has-stored-secret="storedSecrets.remote"
            host-placeholder="host or IP"
            options-placeholder="retryWrites=true&replicaSet=rs0"
            @uri-input="syncFormFromUri('remote')"
            @field-input="rebuildUri('remote')"
            @validate="validateField('remote' + capitalize($event))"
            @save="onSave"
          />
        </div>

        <!-- Local Connection -->
        <div class="border border-[#3c3c3c] rounded-lg p-4 space-y-3">
          <div class="flex items-center justify-between">
            <h4 class="text-xs font-semibold text-[#cccccc] uppercase tracking-wider">Local Connection</h4>
            <label class="flex items-center gap-1.5 text-[10px] text-[#858585] cursor-pointer">
              <input type="checkbox" v-model="form.hasLocal" class="accent-[#007acc]" />
              Enable local sync
            </label>
          </div>

          <template v-if="form.hasLocal">
            <MongoConnectionFields
              v-model="localFields"
              :errors="localErrors"
              :is-srv="isLocalSrv"
              :has-stored-secret="storedSecrets.local"
              host-placeholder="localhost"
              options-placeholder="retryWrites=true"
              @uri-input="syncFormFromUri('local')"
              @field-input="rebuildUri('local')"
              @validate="validateField('local' + capitalize($event))"
              @save="onSave"
            />
          </template>

          <p v-else class="text-[10px] text-[#6e6e6e]">
            Leave disabled to use dump/restore to files instead of live sync.
          </p>
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
          {{ loading ? 'Saving...' : (isEditing ? 'Save' : 'Add') }}
        </button>
      </div>
  </ModalShell>
</template>

<script setup>
import { ref, watch, computed, nextTick, onUnmounted } from 'vue'
import { Loader2 } from 'lucide-vue-next'
import { parseMongoUri, buildMongoUri, parseUriToForm } from '../composables/useMongoUri.js'
import ModalShell from './ModalShell.vue'
import MongoConnectionFields from './MongoConnectionFields.vue'
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
  remoteUri: '',
  remoteHost: '',
  remotePort: 27017,
  remoteUsername: '',
  remotePassword: '',
  remoteDatabase: '',
  remoteAuthSource: 'admin',
  remoteOptions: '',
  hasLocal: false,
  localUri: '',
  localHost: '',
  localPort: 27017,
  localUsername: '',
  localPassword: '',
  localDatabase: '',
  localAuthSource: 'admin',
  localOptions: '',
})

const form = ref(defaultForm())
const errors = ref({})

/** SRV seedlist or multi-host URIs have no single host/port to require. */
function isSrvLikeUri(uri) {
  const trimmed = uri.trim()
  return trimmed.startsWith('mongodb+srv://') || /^mongodb:\/\/[^/]*,/.test(trimmed)
}
const isRemoteSrv = computed(() => isSrvLikeUri(form.value.remoteUri))
const isLocalSrv = computed(() => isSrvLikeUri(form.value.localUri))

const FIELD_KEYS = ['uri', 'host', 'port', 'username', 'password', 'database', 'authSource', 'options']

function capitalize(s) {
  return s.charAt(0).toUpperCase() + s.slice(1)
}

/** Two-way view of one side's flat form keys (remoteUri, remoteHost, ...) as one object. */
function sideFields(prefix) {
  return computed({
    get: () => Object.fromEntries(FIELD_KEYS.map(key => [key, form.value[prefix + capitalize(key)]])),
    set: (value) => {
      for (const key of FIELD_KEYS) {
        form.value[prefix + capitalize(key)] = value[key]
      }
    },
  })
}

function sideErrors(prefix) {
  return computed(() =>
    Object.fromEntries(FIELD_KEYS.map(key => [key, errors.value[prefix + capitalize(key)]])),
  )
}

const remoteFields = sideFields('remote')
const localFields = sideFields('local')
const remoteErrors = sideErrors('remote')
const localErrors = sideErrors('local')

function resetForm() {
  errors.value = {}

  if (props.host) {
    const remote = parseMongoUri(props.host.mongo_uri)
    const local = parseMongoUri(props.host.mongo_local_uri)

    const remoteUri = remote.mode === 'form'
      ? buildMongoUri({
          scheme: 'mongodb',
          host: remote.host,
          port: remote.port,
          username: remote.username,
          password: remote.password,
          database: remote.database,
          authSource: remote.authSource,
          options: remote.options,
        })
      : remote.uri || ''

    const localUri = local.mode === 'form'
      ? buildMongoUri({
          scheme: 'mongodb',
          host: local.host,
          port: local.port,
          username: local.username,
          password: local.password,
          database: local.database,
          authSource: local.authSource,
          options: local.options,
        })
      : local.uri || ''

    form.value = {
      name: props.host.name || '',
      remoteUri,
      remoteHost: remote.mode === 'form' ? remote.host : '',
      remotePort: remote.mode === 'form' ? remote.port : 27017,
      remoteUsername: remote.mode === 'form' ? remote.username : '',
      remotePassword: remote.mode === 'form' ? remote.password : '',
      remoteDatabase: remote.mode === 'form' ? remote.database : '',
      remoteAuthSource: remote.mode === 'form' ? remote.authSource : 'admin',
      remoteOptions: remote.mode === 'form' ? remote.options : '',
      hasLocal: !!props.host.mongo_local_uri,
      localUri,
      localHost: local.mode === 'form' ? local.host : '',
      localPort: local.mode === 'form' ? local.port : 27017,
      localUsername: local.mode === 'form' ? local.username : '',
      localPassword: local.mode === 'form' ? local.password : '',
      localDatabase: local.mode === 'form' ? local.database : '',
      localAuthSource: local.mode === 'form' ? local.authSource : 'admin',
      localOptions: local.mode === 'form' ? local.options : '',
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
    case 'remoteUri':
      if (!val || String(val).trim() === '') {
        msg = 'Remote URI is required'
      } else if (!/^mongodb(\+srv)?:\/\//.test(String(val).trim())) {
        msg = 'URI must start with mongodb:// or mongodb+srv://'
      }
      break
    case 'remoteHost':
      if (!isRemoteSrv.value && (!val || String(val).trim() === '')) msg = 'Host is required'
      break
    case 'remotePort':
      if (!isRemoteSrv.value) {
        if (val === '' || val === null || val === undefined) msg = 'Port is required'
        else if (!Number.isInteger(Number(val))) msg = 'Port must be an integer'
        else if (Number(val) < 1 || Number(val) > 65535) msg = 'Port must be 1–65535'
      }
      break
    case 'remoteDatabase':
      // Database is optional; the app can list all databases when no DB is specified.
      break
    case 'localUri':
      if (form.value.hasLocal) {
        if (!val || String(val).trim() === '') msg = 'Local URI is required'
        else if (!/^mongodb(\+srv)?:\/\//.test(String(val).trim())) {
          msg = 'URI must start with mongodb:// or mongodb+srv://'
        }
      }
      break
    case 'localHost':
      if (form.value.hasLocal && !isLocalSrv.value && (!val || String(val).trim() === '')) {
        msg = 'Host is required'
      }
      break
    case 'localPort':
      if (form.value.hasLocal && !isLocalSrv.value) {
        if (val === '' || val === null || val === undefined) msg = 'Port is required'
        else if (!Number.isInteger(Number(val))) msg = 'Port must be an integer'
        else if (Number(val) < 1 || Number(val) > 65535) msg = 'Port must be 1–65535'
      }
      break
    case 'localDatabase':
      // Database is optional.
      break
  }

  if (msg) errors.value[field] = msg
  else delete errors.value[field]
}

function validateAll() {
  ;['name', 'remoteUri', 'remoteHost', 'remotePort'].forEach(validateField)
  if (form.value.hasLocal) {
    ;['localUri', 'localHost', 'localPort'].forEach(validateField)
  }
  return Object.keys(errors.value).length === 0
}

// While a side's fields are being filled from its URI, ignore rebuild requests
// so the URI the user typed is not rewritten under them.
const syncingFromUri = { remote: false, local: false }

/** Fill one side's structured fields from its connection string. */
function syncFormFromUri(prefix) {
  const parsed = parseUriToForm(form.value[prefix + 'Uri'])
  if (!parsed) return

  syncingFromUri[prefix] = true
  form.value[prefix + 'Host'] = parsed.host
  form.value[prefix + 'Port'] = parsed.port
  form.value[prefix + 'Username'] = parsed.username
  form.value[prefix + 'Password'] = parsed.password
  form.value[prefix + 'Database'] = parsed.database
  form.value[prefix + 'AuthSource'] = parsed.authSource
  form.value[prefix + 'Options'] = parsed.options
  nextTick(() => {
    syncingFromUri[prefix] = false
  })
}

/** Rebuild one side's connection string from its structured fields. */
function rebuildUri(prefix) {
  if (syncingFromUri[prefix]) return
  form.value[prefix + 'Uri'] = buildMongoUri({
    scheme: 'mongodb',
    host: form.value[prefix + 'Host'],
    port: form.value[prefix + 'Port'],
    username: form.value[prefix + 'Username'],
    password: form.value[prefix + 'Password'],
    database: form.value[prefix + 'Database'],
    authSource: form.value[prefix + 'AuthSource'],
    options: form.value[prefix + 'Options'],
  })
}

async function onSave() {
  if (!validateAll()) return

  loading.value = true
  try {
    emit('save', {
      id: props.host?.id ?? null,
      name: form.value.name.trim(),
      mongo_uri: form.value.remoteUri.trim(),
      mongo_local_uri: form.value.hasLocal ? form.value.localUri.trim() || null : null,
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
    loadStoredSecrets()
    window.addEventListener('keydown', onKeydown)
  } else {
    window.removeEventListener('keydown', onKeydown)
  }
})

/**
 * Whether each side already has a password in the keyring.
 *
 * The password itself is never sent to the frontend, so the field renders empty
 * on edit; without this the user cannot tell "no password" from "not shown".
 */
const storedSecrets = ref({ remote: false, local: false })

async function loadStoredSecrets() {
  storedSecrets.value = { remote: false, local: false }
  const hostId = props.host?.id
  if (!hostId) return
  for (const side of ['remote', 'local']) {
    try {
      storedSecrets.value[side] = await invoke('mongodb_has_secret', { hostId, side })
    } catch {
      // Not knowing is not worth blocking the dialog over.
      storedSecrets.value[side] = false
    }
  }
}

onUnmounted(() => {
  window.removeEventListener('keydown', onKeydown)
})
</script>
