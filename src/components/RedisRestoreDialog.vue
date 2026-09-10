<template>
  <ModalShell :show="state.show" z="z-[100]" panel-class="p-6 w-[32rem] shadow-xl">
    <h3 class="text-base font-medium text-ink mb-1">Restore into {{ connectionName }}</h3>
    <p class="text-xs text-ink-2 mb-4 break-all">{{ state.path }}</p>

    <div v-if="state.header" class="bg-canvas rounded p-3 mb-4 text-xs space-y-1">
      <div class="flex justify-between">
        <span class="text-ink-2">Taken from</span>
        <span class="text-ink">{{ state.header.source }}</span>
      </div>
      <div class="flex justify-between">
        <span class="text-ink-2">Redis version</span>
        <span class="text-ink">{{ state.header.redis_version }}</span>
      </div>
      <div class="flex justify-between">
        <span class="text-ink-2">Created</span>
        <span class="text-ink">{{ state.header.created_at }}</span>
      </div>
      <div v-if="state.header.pattern" class="flex justify-between">
        <span class="text-ink-2">Pattern</span>
        <span class="text-ink font-mono">{{ state.header.pattern }}</span>
      </div>
    </div>

    <p v-if="warning" class="text-xs text-warn-soft mb-4">{{ warning }}</p>

    <label class="block text-xs text-ink-2 mb-1">Restore into database</label>
    <div class="mb-4">
      <SelectMenu
        block
        :modelValue="state.targetDb"
        :options="dbOptions"
        @update:modelValue="$emit('update:targetDb', $event)"
      />
    </div>

    <label class="flex items-start gap-2 mb-3 cursor-pointer">
      <input
        type="checkbox"
        :checked="state.replace"
        @change="$emit('update:replace', $event.target.checked)"
        class="accent-accent mt-0.5"
      />
      <span class="text-xs text-ink">
        Overwrite keys that already exist
        <span class="block text-2xs text-ink-2">
          Without this, a key that is already there is left alone and counted as skipped.
        </span>
      </span>
    </label>

    <label class="flex items-start gap-2 mb-5 cursor-pointer">
      <input
        type="checkbox"
        :checked="state.flushFirst"
        @change="$emit('update:flushFirst', $event.target.checked)"
        class="accent-bad mt-0.5"
      />
      <span class="text-xs text-ink">
        Empty the database first
        <span class="block text-2xs text-bad">
          Deletes every key in db{{ state.targetDb }} before restoring. This cannot be undone.
        </span>
      </span>
    </label>

    <div class="flex justify-end gap-2">
      <button
        @click="$emit('cancel')"
        class="px-3 py-1.5 text-xs rounded text-ink hover:bg-input"
      >
        Cancel
      </button>
      <button
        @click="$emit('confirm')"
        class="px-3 py-1.5 text-xs rounded text-white"
        :class="state.flushFirst ? 'bg-bad-solid-hover hover:bg-bad-solid' : 'bg-accent-solid hover:bg-accent-solid-hover'"
      >
        {{ state.flushFirst ? 'Empty and restore' : 'Restore' }}
      </button>
    </div>
  </ModalShell>
</template>

<script setup>
/**
 * Confirming a restore.
 *
 * Shows what the file says about itself before anything is written, because
 * "which backup is this and where is it about to go" is the question the user
 * actually has at this moment.
 */
import { computed } from 'vue'
import ModalShell from './ModalShell.vue'
import SelectMenu from './SelectMenu.vue'
import { restoreWarning } from '../utils/redisBackup.js'

const props = defineProps({
  state: { type: Object, required: true },
  connectionName: { type: String, default: '' },
  serverInfo: { type: Object, default: null },
})

defineEmits(['confirm', 'cancel', 'update:replace', 'update:flushFirst', 'update:targetDb'])

const warning = computed(() => restoreWarning(props.state.header, props.serverInfo))

const dbOptions = Array.from({ length: 16 }, (_, i) => ({ value: i, label: `db${i}` }))
</script>
