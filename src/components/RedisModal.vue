<template>
  <ModalShell
    :show="show"
    panel-class="p-6 w-[30rem] shadow-xl max-h-[90vh] overflow-y-auto"
  >
    <h3 class="text-base font-medium text-ink mb-4">
      {{ host ? 'Edit' : 'Add' }} Redis connection
    </h3>

    <div class="mb-4">
      <label class="block text-xs text-ink-2 mb-1">Name</label>
      <input
        v-model="form.name"
        placeholder="Production cache"
        class="w-full bg-input text-ink text-xs rounded px-2 py-1.5 outline-none focus:ring-1 focus:ring-accent"
      />
      <p v-if="errors.name" class="text-[10px] text-bad mt-1">{{ errors.name }}</p>
    </div>

    <fieldset class="border border-line rounded p-3 mb-5">
      <legend class="text-xs text-ink-2 px-1">Connection</legend>
      <!--
        Not `v-model`: `form` is a const reactive object, so reassigning it is
        what the compiler warns about. Merging keeps the same object, which is
        also what the field watchers are bound to.
      -->
      <RedisConnectionFields
        :modelValue="form"
        @update:modelValue="Object.assign(form, $event)"
        v-model:tunnelHostId="tunnelHostId"
        :errors="errors"
        :hasStoredSecret="hasStoredSecret"
        :sshHosts="sshHosts"
        @uri-input="syncFormFromUri"
        @field-input="rebuildUri"
      />
    </fieldset>

    <div class="flex justify-end gap-2">
      <button
        @click="$emit('close')"
        class="px-3 py-1.5 text-xs rounded text-ink hover:bg-input"
      >
        Cancel
      </button>
      <button
        @click="submit"
        class="px-3 py-1.5 text-xs rounded bg-accent-solid hover:bg-accent-solid-hover text-white"
      >
        Save
      </button>
    </div>
  </ModalShell>
</template>

<script setup>
/**
 * Add or edit a Redis connection.
 *
 * The URI and the individual fields stay in step in both directions. The
 * `syncingFromUri` guard is what stops the URI being rewritten under the
 * user's cursor while they type it — the same flag `MongoDbModal` needs.
 */
import { ref, reactive, watch } from 'vue'
import ModalShell from './ModalShell.vue'
import RedisConnectionFields from './RedisConnectionFields.vue'
import { invoke } from '../utils/invoke.js'
import { parseRedisUri, buildRedisUri, isRedisUri, isTlsRedisUri } from '../utils/redisUri.js'
import { hostKind, HOST_KIND } from '../utils/hostKind.js'
import { useConnectionStore } from '../stores/connection.js'

const props = defineProps({
  show: { type: Boolean, default: false },
  host: { type: Object, default: null },
})

const emit = defineEmits(['save', 'close'])

const store = useConnectionStore()
const sshHosts = ref([])
const tunnelHostId = ref(null)
const hasStoredSecret = ref(false)
const errors = reactive({})

const form = reactive({
  name: '',
  uri: '',
  scheme: 'redis',
  host: '',
  port: '',
  database: '',
  username: '',
  password: '',
})

let syncingFromUri = false

function syncFormFromUri() {
  syncingFromUri = true
  const parsed = parseRedisUri(form.uri)
  Object.assign(form, parsed)
  syncingFromUri = false
}

function rebuildUri() {
  if (syncingFromUri) return
  form.uri = buildRedisUri(form)
}

function validate() {
  Object.keys(errors).forEach(k => delete errors[k])
  if (!form.name.trim()) errors.name = 'A name is required'
  if (!form.uri.trim()) errors.uri = 'A connection URI is required'
  else if (!isRedisUri(form.uri)) errors.uri = 'Must start with redis:// or rediss://'
  else if (isTlsRedisUri(form.uri) && tunnelHostId.value != null) {
    errors.uri = 'A TLS connection cannot be tunnelled — turn TLS off, or connect directly'
  }
  return Object.keys(errors).length === 0
}

function submit() {
  if (!validate()) return
  emit('save', {
    id: props.host?.id ?? null,
    name: form.name.trim(),
    redis_uri: form.uri.trim(),
    redis_tunnel_host_id: tunnelHostId.value,
  })
}

watch(
  () => props.show,
  async shown => {
    if (!shown) return
    sshHosts.value = store.hosts.filter(h => hostKind(h) === HOST_KIND.SSH)

    const existing = props.host
    Object.assign(form, {
      name: existing?.name ?? '',
      uri: existing?.redis_uri ?? '',
      scheme: 'redis',
      host: '',
      port: '',
      database: '',
      username: '',
      password: '',
    })
    syncFormFromUri()
    tunnelHostId.value = existing?.redis_tunnel_host_id ?? null

    hasStoredSecret.value = existing?.id
      ? await invoke('redis_has_secret', { hostId: existing.id }).catch(() => false)
      : false
  },
  { immediate: true },
)
</script>
