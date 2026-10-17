<template>
  <div class="space-y-4">
    <div>
      <label class="block text-xs text-ink-2 mb-1.5">
        Address <span class="text-bad">*</span>
      </label>
      <input
        :value="target"
        @input="$emit('update:target', $event.target.value)"
        placeholder="10.0.0.5:1"
        :class="inputClass('target')"
      />
      <p v-if="errors.target" class="text-xs text-bad mt-1">{{ errors.target }}</p>
      <p v-else class="text-2xs text-ink-3 mt-1">
        <code>:1</code> means display 1 (port 5901). Use <code>::5901</code> for a literal port.
      </p>
    </div>

    <div class="flex gap-3">
      <div class="flex-1">
        <label class="block text-xs text-ink-2 mb-1.5">
          Password
          <span v-if="hasStoredSecret" class="text-ink-3">(stored)</span>
        </label>
        <input
          :value="password"
          @input="$emit('update:password', $event.target.value)"
          type="password"
          autocomplete="off"
          :placeholder="hasStoredSecret ? 'Leave blank to keep' : ''"
          :class="inputClass('password')"
        />
      </div>
      <div class="flex-1">
        <label class="block text-xs text-ink-2 mb-1.5">
          User name
          <span class="text-ink-3">(macOS)</span>
        </label>
        <input
          :value="username"
          @input="$emit('update:username', $event.target.value)"
          autocomplete="off"
          :class="inputClass('username')"
        />
      </div>
    </div>

    <!-- Only macOS Screen Sharing has anywhere to put a user name, so saying
         so beats leaving people guessing whether to fill it in. -->
    <div v-if="showTunnel">
      <label class="block text-xs text-ink-2 mb-1.5">Connect through</label>
      <SelectMenu
        block
        size="md"
        :model-value="tunnelHostId"
        :options="tunnelOptions"
        @update:model-value="$emit('update:tunnelHostId', $event)"
      />
      <p class="text-2xs text-ink-3 mt-1">{{ tunnelHint }}</p>
    </div>

    <label class="flex items-center gap-2 text-sm text-ink-2">
      <input
        type="checkbox"
        :checked="viewOnly"
        @change="$emit('update:viewOnly', $event.target.checked)"
        class="accent-accent"
      />
      View only — never send keyboard or mouse input
    </label>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import SelectMenu from './SelectMenu.vue'

const props = defineProps({
  target: { type: String, default: '' },
  password: { type: String, default: '' },
  username: { type: String, default: '' },
  tunnelHostId: { type: [Number, null], default: null },
  viewOnly: { type: Boolean, default: false },
  hasStoredSecret: { type: Boolean, default: false },
  errors: { type: Object, default: () => ({}) },
  sshHosts: { type: Array, default: () => [] },
  /** True when this row is itself an SSH host, which changes the default. */
  isSshHost: { type: Boolean, default: false },
  showTunnel: { type: Boolean, default: true },
})

defineEmits([
  'update:target',
  'update:password',
  'update:username',
  'update:tunnelHostId',
  'update:viewOnly',
])

/**
 * The tunnel choices.
 *
 * On an SSH host the default is *this host* rather than "directly", because a
 * VNC server almost always binds 127.0.0.1 and would not be reachable any other
 * way — so a direct attempt is the wrong thing to try first.
 */
const tunnelOptions = computed(() => [
  {
    value: null,
    label: props.isSshHost ? 'This host’s SSH connection' : 'Directly',
    hint: props.isSshHost ? 'recommended' : undefined,
  },
  ...props.sshHosts.map(h => ({
    value: h.id,
    label: h.name,
    hint: `${h.username}@${h.host}`,
  })),
])

/** The same input treatment the other connection dialogs use. */
function inputClass(key) {
  const base =
    'w-full bg-input border rounded-sm px-3 py-2 text-sm text-ink focus:outline-hidden transition-colors'
  const error = props.errors[key] ? 'border-bad focus:border-bad' : 'border-line focus:border-accent'
  return `${base} ${error}`
}

const tunnelHint = computed(() => {
  if (props.tunnelHostId != null) return 'The address above is resolved on that host.'
  return props.isSshHost
    ? 'The address above is resolved on this host — usually 127.0.0.1.'
    : 'The VNC server must be reachable from this machine.'
})
</script>
