<template>
  <div
    v-if="show"
    class="fixed inset-0 bg-black/60 flex items-center justify-center z-50"
  >
    <div class="bg-[#252526] rounded-lg p-6 w-[28rem] border border-[#3c3c3c] shadow-xl max-h-[90vh] overflow-y-auto">
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

          <div>
            <label class="block text-xs text-[#858585] mb-1.5">
              Connection String <span class="text-[#f44336]">*</span>
            </label>
            <input
              v-model="form.remoteUri"
              type="text"
              placeholder="mongodb:// or mongodb+srv://"
              :class="inputClass('remoteUri')"
              @input="syncRemoteFormFromUri"
              @blur="validateField('remoteUri')"
              @keydown.enter="onSave"
            />
            <p v-if="errors.remoteUri" class="text-xs text-[#f44336] mt-1">{{ errors.remoteUri }}</p>
          </div>

          <div class="flex gap-3">
            <div class="flex-[2]">
              <label class="block text-xs text-[#858585] mb-1.5">
                Host <span v-if="!isRemoteSrv" class="text-[#f44336]">*</span>
              </label>
              <input
                v-model="form.remoteHost"
                type="text"
                placeholder="host or IP"
                :class="inputClass('remoteHost')"
                @input="rebuildRemoteUri"
                @blur="validateField('remoteHost')"
                @keydown.enter="onSave"
              />
              <p v-if="errors.remoteHost" class="text-xs text-[#f44336] mt-1">{{ errors.remoteHost }}</p>
            </div>
            <div class="flex-1">
              <label class="block text-xs text-[#858585] mb-1.5">
                Port <span v-if="!isRemoteSrv" class="text-[#f44336]">*</span>
              </label>
              <input
                v-model.number="form.remotePort"
                type="number"
                placeholder="27017"
                :class="inputClass('remotePort')"
                @input="rebuildRemoteUri"
                @blur="validateField('remotePort')"
                @keydown.enter="onSave"
              />
              <p v-if="errors.remotePort" class="text-xs text-[#f44336] mt-1">{{ errors.remotePort }}</p>
            </div>
          </div>

          <div class="flex gap-3">
            <div class="flex-1">
              <label class="block text-xs text-[#858585] mb-1.5">Username</label>
              <input
                v-model="form.remoteUsername"
                type="text"
                placeholder="user"
                :class="inputClass('remoteUsername')"
                @input="rebuildRemoteUri"
                @keydown.enter="onSave"
              />
            </div>
            <div class="flex-1">
              <label class="block text-xs text-[#858585] mb-1.5">Password</label>
              <div class="relative">
                <input
                  v-model="form.remotePassword"
                  :type="showRemotePassword ? 'text' : 'password'"
                  placeholder="password"
                  :class="[inputClass('remotePassword'), 'pr-8']"
                  @input="rebuildRemoteUri"
                  @keydown.enter="onSave"
                />
                <button
                  type="button"
                  @click="showRemotePassword = !showRemotePassword"
                  class="absolute right-2 top-1/2 -translate-y-1/2 text-[#6e6e6e] hover:text-[#cccccc]"
                >
                  <component :is="showRemotePassword ? EyeOff : Eye" :size="14" />
                </button>
              </div>
            </div>
          </div>

          <div class="flex gap-3">
            <div class="flex-1">
              <label class="block text-xs text-[#858585] mb-1.5">
                Database
              </label>
              <input
                v-model="form.remoteDatabase"
                type="text"
                placeholder="database"
                :class="inputClass('remoteDatabase')"
                @input="rebuildRemoteUri"
                @blur="validateField('remoteDatabase')"
                @keydown.enter="onSave"
              />
              <p v-if="errors.remoteDatabase" class="text-xs text-[#f44336] mt-1">{{ errors.remoteDatabase }}</p>
            </div>
            <div class="flex-1">
              <label class="block text-xs text-[#858585] mb-1.5">Auth Source</label>
              <input
                v-model="form.remoteAuthSource"
                type="text"
                placeholder="admin"
                :class="inputClass('remoteAuthSource')"
                @input="rebuildRemoteUri"
                @keydown.enter="onSave"
              />
            </div>
          </div>

          <div>
            <label class="block text-xs text-[#858585] mb-1.5">Connection Options</label>
            <input
              v-model="form.remoteOptions"
              type="text"
              placeholder="retryWrites=true&replicaSet=rs0"
              :class="inputClass('remoteOptions')"
              @input="rebuildRemoteUri"
              @keydown.enter="onSave"
            />
          </div>
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
            <div>
              <label class="block text-xs text-[#858585] mb-1.5">
                Connection String <span class="text-[#f44336]">*</span>
              </label>
              <input
                v-model="form.localUri"
                type="text"
                placeholder="mongodb:// or mongodb+srv://"
                :class="inputClass('localUri')"
                @input="syncLocalFormFromUri"
                @blur="validateField('localUri')"
                @keydown.enter="onSave"
              />
              <p v-if="errors.localUri" class="text-xs text-[#f44336] mt-1">{{ errors.localUri }}</p>
            </div>

            <div class="flex gap-3">
              <div class="flex-[2]">
                <label class="block text-xs text-[#858585] mb-1.5">
                  Host <span v-if="!isLocalSrv" class="text-[#f44336]">*</span>
                </label>
                <input
                  v-model="form.localHost"
                  type="text"
                  placeholder="localhost"
                  :class="inputClass('localHost')"
                  @input="rebuildLocalUri"
                  @blur="validateField('localHost')"
                  @keydown.enter="onSave"
                />
                <p v-if="errors.localHost" class="text-xs text-[#f44336] mt-1">{{ errors.localHost }}</p>
              </div>
              <div class="flex-1">
                <label class="block text-xs text-[#858585] mb-1.5">
                  Port <span v-if="!isLocalSrv" class="text-[#f44336]">*</span>
                </label>
                <input
                  v-model.number="form.localPort"
                  type="number"
                  placeholder="27017"
                  :class="inputClass('localPort')"
                  @input="rebuildLocalUri"
                  @blur="validateField('localPort')"
                  @keydown.enter="onSave"
                />
                <p v-if="errors.localPort" class="text-xs text-[#f44336] mt-1">{{ errors.localPort }}</p>
              </div>
            </div>

            <div class="flex gap-3">
              <div class="flex-1">
                <label class="block text-xs text-[#858585] mb-1.5">Username</label>
                <input
                  v-model="form.localUsername"
                  type="text"
                  placeholder="user"
                  :class="inputClass('localUsername')"
                  @input="rebuildLocalUri"
                  @keydown.enter="onSave"
                />
              </div>
              <div class="flex-1">
                <label class="block text-xs text-[#858585] mb-1.5">Password</label>
                <div class="relative">
                  <input
                    v-model="form.localPassword"
                    :type="showLocalPassword ? 'text' : 'password'"
                    placeholder="password"
                    :class="[inputClass('localPassword'), 'pr-8']"
                    @input="rebuildLocalUri"
                    @keydown.enter="onSave"
                  />
                  <button
                    type="button"
                    @click="showLocalPassword = !showLocalPassword"
                    class="absolute right-2 top-1/2 -translate-y-1/2 text-[#6e6e6e] hover:text-[#cccccc]"
                  >
                    <component :is="showLocalPassword ? EyeOff : Eye" :size="14" />
                  </button>
                </div>
              </div>
            </div>

            <div class="flex gap-3">
              <div class="flex-1">
                <label class="block text-xs text-[#858585] mb-1.5">
                  Database
                </label>
                <input
                  v-model="form.localDatabase"
                  type="text"
                  placeholder="database"
                  :class="inputClass('localDatabase')"
                  @input="rebuildLocalUri"
                  @blur="validateField('localDatabase')"
                  @keydown.enter="onSave"
                />
                <p v-if="errors.localDatabase" class="text-xs text-[#f44336] mt-1">{{ errors.localDatabase }}</p>
              </div>
              <div class="flex-1">
                <label class="block text-xs text-[#858585] mb-1.5">Auth Source</label>
                <input
                  v-model="form.localAuthSource"
                  type="text"
                  placeholder="admin"
                  :class="inputClass('localAuthSource')"
                  @input="rebuildLocalUri"
                  @keydown.enter="onSave"
                />
              </div>
            </div>

            <div>
              <label class="block text-xs text-[#858585] mb-1.5">Connection Options</label>
              <input
                v-model="form.localOptions"
                type="text"
                placeholder="retryWrites=true"
                :class="inputClass('localOptions')"
                @input="rebuildLocalUri"
                @keydown.enter="onSave"
              />
            </div>
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
    </div>
  </div>
</template>

<script setup>
import { ref, watch, computed, nextTick } from 'vue'
import { Loader2, Eye, EyeOff } from 'lucide-vue-next'
import { parseMongoUri, buildMongoUri, parseUriToForm } from '../composables/useMongoUri.js'

const props = defineProps({
  show: Boolean,
  host: Object,
})

const emit = defineEmits(['save', 'close'])

const isEditing = computed(() => !!props.host)

const nameInput = ref(null)
const loading = ref(false)
const showRemotePassword = ref(false)
const showLocalPassword = ref(false)

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

const isRemoteSrv = computed(() =>
  form.value.remoteUri.trim().startsWith('mongodb+srv://') ||
  /^mongodb:\/\/[^/]*,/.test(form.value.remoteUri.trim())
)
const isLocalSrv = computed(() =>
  form.value.localUri.trim().startsWith('mongodb+srv://') ||
  /^mongodb:\/\/[^/]*,/.test(form.value.localUri.trim())
)

function resetForm() {
  errors.value = {}
  showRemotePassword.value = false
  showLocalPassword.value = false

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

let syncingFromRemoteUri = false
function syncRemoteFormFromUri() {
  const parsed = parseUriToForm(form.value.remoteUri)
  if (!parsed) return

  syncingFromRemoteUri = true
  form.value.remoteHost = parsed.host
  form.value.remotePort = parsed.port
  form.value.remoteUsername = parsed.username
  form.value.remotePassword = parsed.password
  form.value.remoteDatabase = parsed.database
  form.value.remoteAuthSource = parsed.authSource
  form.value.remoteOptions = parsed.options
  nextTick(() => {
    syncingFromRemoteUri = false
  })
}

function rebuildRemoteUri() {
  if (syncingFromRemoteUri) return
  form.value.remoteUri = buildMongoUri({
    scheme: 'mongodb',
    host: form.value.remoteHost,
    port: form.value.remotePort,
    username: form.value.remoteUsername,
    password: form.value.remotePassword,
    database: form.value.remoteDatabase,
    authSource: form.value.remoteAuthSource,
    options: form.value.remoteOptions,
  })
}

let syncingFromLocalUri = false
function syncLocalFormFromUri() {
  const parsed = parseUriToForm(form.value.localUri)
  if (!parsed) return

  syncingFromLocalUri = true
  form.value.localHost = parsed.host
  form.value.localPort = parsed.port
  form.value.localUsername = parsed.username
  form.value.localPassword = parsed.password
  form.value.localDatabase = parsed.database
  form.value.localAuthSource = parsed.authSource
  form.value.localOptions = parsed.options
  nextTick(() => {
    syncingFromLocalUri = false
  })
}

function rebuildLocalUri() {
  if (syncingFromLocalUri) return
  form.value.localUri = buildMongoUri({
    scheme: 'mongodb',
    host: form.value.localHost,
    port: form.value.localPort,
    username: form.value.localUsername,
    password: form.value.localPassword,
    database: form.value.localDatabase,
    authSource: form.value.localAuthSource,
    options: form.value.localOptions,
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
    window.addEventListener('keydown', onKeydown)
  } else {
    window.removeEventListener('keydown', onKeydown)
  }
})
</script>
