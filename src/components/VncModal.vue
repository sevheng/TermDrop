<template>
  <ModalShell
    @close="$emit('close')" :show="show" dim="bg-black/60" z="z-50" panel-class="p-6 w-md shadow-xl max-h-[90vh] overflow-y-auto">
      <h3 class="text-lg font-semibold text-ink mb-5">
        {{ isEditing ? 'Edit remote desktop' : 'Add remote desktop' }}
      </h3>

      <div class="space-y-4">
        <!-- Name -->
        <div>
          <label class="block text-xs text-ink-2 mb-1.5">
            Name <span class="text-bad">*</span>
          </label>
          <input
            v-model="form.name"
            type="text"
            placeholder="Kiosk screen"
            :class="inputClass('name')"
          />
          <p v-if="errors.name" class="text-xs text-bad mt-1">{{ errors.name }}</p>
        </div>

        <VncConnectionFields
          v-model:target="form.target"
          v-model:password="form.password"
          v-model:username="form.username"
          v-model:tunnelHostId="form.tunnelHostId"
          v-model:viewOnly="form.viewOnly"
          :has-stored-secret="hasStoredSecret"
          :errors="errors"
          :ssh-hosts="sshHosts"
        />
      </div>

      <!-- Actions -->
      <div class="flex justify-end gap-2 mt-6">
        <button
          @click="$emit('close')"
          class="px-4 py-2 text-sm text-ink-2 hover:text-ink rounded-sm hover:bg-raised transition-colors"
        >
          Cancel
        </button>
        <button
          @click="submit"
          class="px-4 py-2 text-sm text-white rounded-sm bg-accent-solid hover:bg-accent-solid-hover transition-colors"
        >
          {{ isEditing ? 'Save' : 'Add' }}
        </button>
      </div>
  </ModalShell>
</template>

<script setup>
import { computed, reactive, ref, watch } from 'vue'
import { invoke } from '../utils/invoke.js'
import { useConnectionStore } from '../stores/connection.js'
import { hostKind, HOST_KIND } from '../utils/hostKind.js'
import { parseVncTarget, vncDisplayTarget } from '../utils/vncTarget.js'
import ModalShell from './ModalShell.vue'
import VncConnectionFields from './VncConnectionFields.vue'

const props = defineProps({
  show: Boolean,
  host: { type: Object, default: null },
})
const emit = defineEmits(['save', 'close'])

const store = useConnectionStore()
const isEditing = computed(() => !!props.host)

/** The same input treatment every other connection dialog uses. */
function inputClass(key) {
  const base =
    'w-full bg-input border rounded-sm px-3 py-2 text-sm text-ink focus:outline-hidden transition-colors'
  const error = errors[key] ? 'border-bad focus:border-bad' : 'border-line focus:border-accent'
  return `${base} ${error}`
}

// Mutated with Object.assign and never reassigned, so the template's bindings
// keep pointing at the same object.
const form = reactive({
  name: '',
  target: '',
  password: '',
  username: '',
  tunnelHostId: null,
  viewOnly: false,
})
const errors = reactive({})
const sshHosts = ref([])
const hasStoredSecret = ref(false)

function validate() {
  Object.keys(errors).forEach(k => delete errors[k])
  if (!form.name.trim()) errors.name = 'A name is required'
  const parsed = parseVncTarget(form.target)
  if (parsed.error) errors.target = parsed.error
  return Object.keys(errors).length === 0
}

function submit() {
  if (!validate()) return
  const { host, port } = parseVncTarget(form.target)
  emit('save', {
    id: props.host?.id ?? null,
    name: form.name.trim(),
    vnc_host: host,
    vnc_port: port,
    vnc_username: form.username.trim() || null,
    vnc_tunnel_host_id: form.tunnelHostId,
    vnc_view_only: form.viewOnly ? 1 : 0,
    password: form.password,
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
      target: existing ? vncDisplayTarget(existing) : '',
      password: '',
      username: existing?.vnc_username ?? '',
      tunnelHostId: existing?.vnc_tunnel_host_id ?? null,
      viewOnly: existing?.vnc_view_only === 1,
    })
    Object.keys(errors).forEach(k => delete errors[k])

    hasStoredSecret.value = existing?.id
      ? await invoke('vnc_has_secret', { hostId: existing.id }).catch(() => false)
      : false
  },
  { immediate: true },
)
</script>
