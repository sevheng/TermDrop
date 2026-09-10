<template>
  <div class="space-y-3">
    <div>
      <label class="block text-xs text-ink-2 mb-1">Connection URI</label>
      <input
        v-model="uri"
        @input="$emit('uri-input')"
        placeholder="redis://:password@localhost:6379/0"
        class="w-full bg-input text-ink text-xs rounded px-2 py-1.5 font-mono outline-none focus:ring-1 focus:ring-accent"
      />
      <p v-if="errors.uri" class="text-[10px] text-bad mt-1">{{ errors.uri }}</p>
    </div>

    <div class="flex items-center gap-2">
      <div class="flex-1">
        <label class="block text-xs text-ink-2 mb-1">Host</label>
        <input
          v-model="host"
          @input="$emit('field-input')"
          placeholder="localhost"
          class="w-full bg-input text-ink text-xs rounded px-2 py-1.5 outline-none focus:ring-1 focus:ring-accent"
        />
      </div>
      <div class="w-24">
        <label class="block text-xs text-ink-2 mb-1">Port</label>
        <input
          v-model="port"
          @input="$emit('field-input')"
          placeholder="6379"
          class="w-full bg-input text-ink text-xs rounded px-2 py-1.5 outline-none focus:ring-1 focus:ring-accent"
        />
      </div>
      <div class="w-20">
        <label class="block text-xs text-ink-2 mb-1">DB</label>
        <input
          v-model="database"
          @input="$emit('field-input')"
          placeholder="0"
          class="w-full bg-input text-ink text-xs rounded px-2 py-1.5 outline-none focus:ring-1 focus:ring-accent"
        />
      </div>
    </div>

    <div class="flex items-center gap-2">
      <div class="flex-1">
        <label class="block text-xs text-ink-2 mb-1">
          Username
          <span class="text-ink-3">(ACL, Redis 6+)</span>
        </label>
        <input
          v-model="username"
          @input="$emit('field-input')"
          class="w-full bg-input text-ink text-xs rounded px-2 py-1.5 outline-none focus:ring-1 focus:ring-accent"
        />
      </div>
      <div class="flex-1">
        <label class="block text-xs text-ink-2 mb-1">Password</label>
        <div class="relative">
          <input
            v-model="password"
            :type="showPassword ? 'text' : 'password'"
            @input="$emit('field-input')"
            :placeholder="hasStoredSecret ? '•••••••• (stored)' : ''"
            class="w-full bg-input text-ink text-xs rounded px-2 py-1.5 pr-7 outline-none focus:ring-1 focus:ring-accent"
          />
          <button
            type="button"
            @click="showPassword = !showPassword"
            class="absolute right-1.5 top-1/2 -translate-y-1/2 text-ink-2 hover:text-ink"
          >
            <component :is="showPassword ? EyeOff : Eye" :size="12" />
          </button>
        </div>
      </div>
    </div>

    <label class="flex items-center gap-2 cursor-pointer">
      <input v-model="tls" type="checkbox" @change="$emit('field-input')" class="accent-accent" />
      <span class="text-xs text-ink">Use TLS (rediss://)</span>
    </label>

    <div>
      <label class="block text-xs text-ink-2 mb-1">Connect through SSH host</label>
      <SelectMenu
        block
        :modelValue="tunnelHostId ?? ''"
        :options="tunnelOptions"
        @update:modelValue="$emit('update:tunnelHostId', $event === '' ? null : Number($event))"
      />
      <p class="text-[10px] text-ink-3 mt-1">
        For a Redis that only listens on a private network. The host and port above are
        resolved from the SSH host, not from this machine.
      </p>
      <!--
        Through a tunnel the driver connects to 127.0.0.1, so a certificate
        naming the real host can never verify. SSH already encrypts the hop, so
        the answer is plain redis:// rather than switching verification off.
      -->
      <p v-if="tls && tunnelHostId != null" class="text-[10px] text-bad mt-1">
        A TLS connection cannot be tunnelled: the certificate names the real host, but through
        a tunnel only 127.0.0.1 is visible. The SSH tunnel already encrypts this hop — turn TLS
        off, or connect directly.
      </p>
    </div>
  </div>
</template>

<script setup>
/**
 * The Redis connection form: a URI and the fields that make it up.
 *
 * Presentational — the parent owns the values, the URI/field synchronisation
 * and the validation, exactly as `MongoConnectionFields` does.
 */
import { ref, computed } from 'vue'
import { Eye, EyeOff } from 'lucide-vue-next'
import SelectMenu from './SelectMenu.vue'

const props = defineProps({
  modelValue: { type: Object, required: true },
  errors: { type: Object, default: () => ({}) },
  hasStoredSecret: { type: Boolean, default: false },
  sshHosts: { type: Array, default: () => [] },
  tunnelHostId: { type: Number, default: null },
})

const emit = defineEmits([
  'update:modelValue',
  'update:tunnelHostId',
  'uri-input',
  'field-input',
])

const showPassword = ref(false)

/** The bastion choices, with the address as a hint so two hosts sharing a
 *  name are still tellable apart. */
const tunnelOptions = computed(() => [
  { value: '', label: 'Connect directly' },
  ...props.sshHosts.map(h => ({
    value: h.id,
    label: h.name,
    hint: `${h.username}@${h.host}`,
  })),
])

/** One writable computed per field, so v-model works without a watcher. */
function field(key) {
  return computed({
    get: () => props.modelValue[key],
    set: value => emit('update:modelValue', { ...props.modelValue, [key]: value }),
  })
}

const uri = field('uri')
const host = field('host')
const port = field('port')
const database = field('database')
const username = field('username')
const password = field('password')

const tls = computed({
  get: () => props.modelValue.scheme === 'rediss',
  set: value =>
    emit('update:modelValue', { ...props.modelValue, scheme: value ? 'rediss' : 'redis' }),
})
</script>
