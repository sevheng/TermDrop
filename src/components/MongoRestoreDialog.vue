<template>
  <ModalShell :show="state.show" dim="bg-black/60" z="z-[100]" panel-class="p-5 w-[28rem] shadow-xl">
      <h3 class="text-base font-semibold text-ink mb-3">
        Confirm restore into <span class="text-accent-soft">{{ connectionName }}</span>
      </h3>
      <div class="space-y-2 text-sm text-ink">
        <p>
          Source {{ state.isArchive ? 'archive' : 'folder' }}:
          <span class="font-mono text-good break-all">{{ state.inputPath }}</span>
        </p>
        <div v-if="!state.isArchive && state.sourceDbs.length > 0">
          <p class="text-ink-2 mb-1">Data found in folder:</p>
          <ul class="max-h-32 overflow-y-auto bg-canvas rounded p-2 space-y-1 text-xs">
            <li v-for="db in state.sourceDbs" :key="db.name">
              <span class="text-accent-soft">{{ db.name }}</span>:
              <span class="text-ink">{{ db.collections.map(c => c.name).join(', ') }}</span>
            </li>
          </ul>
        </div>
        <template v-if="state.entries.length > 0">
          <p>
            Target filter database{{ state.entries.length > 1 ? 's' : '' }}:
            <span class="font-mono text-accent-soft">
              {{ state.entries.map(e => e.db).join(', ') }}
            </span>
          </p>
          <div>
            <p class="text-ink-2 mb-1">Collections to restore:</p>
            <ul class="max-h-32 overflow-y-auto bg-canvas rounded p-2 space-y-0.5 text-xs">
              <li v-for="entry in state.entries" :key="entry.db">
                <span class="text-accent-soft">{{ entry.db }}</span>:
                <span class="text-ink">{{ entry.collections.join(', ') }}</span>
              </li>
            </ul>
          </div>
        </template>
        <template v-else-if="!state.isArchive && state.sourceDbs.length === 1">
          <p class="text-accent-soft">
            Will restore database <span class="font-mono text-accent-soft">{{ state.sourceDbs[0].name }}</span>
            (all collections found in the folder).
          </p>
        </template>
        <template v-else-if="!state.isArchive && state.sourceDbs.length > 1">
          <p class="text-accent-soft">
            Will restore all databases found in the folder:
            <span class="font-mono text-accent-soft">
              {{ state.sourceDbs.map(d => d.name).join(', ') }}
            </span>
          </p>
        </template>
        <p v-else class="text-accent-soft">
          No database selected — everything in the source will be restored.
        </p>
        <label class="flex items-start gap-2 pt-1 cursor-pointer">
          <input
            type="checkbox"
            :checked="state.dropFirst"
            @change="$emit('update:dropFirst', $event.target.checked)"
            class="accent-bad mt-0.5 shrink-0"
          />
          <span class="text-xs">
            Drop existing collections in the target first
          </span>
        </label>
        <p v-if="state.dropFirst" class="text-bad text-xs">
          Existing collections in the target database will be dropped before
          restoring. This cannot be undone.
        </p>
        <p v-else class="text-ink-2 text-xs">
          Existing documents are kept. Documents whose <span class="font-mono">_id</span>
          already exists will be reported as failures, not overwritten.
        </p>
      </div>
      <div class="flex justify-end gap-2 mt-5">
        <button
          @click="$emit('cancel')"
          class="px-3 py-1.5 text-sm text-ink-2 hover:text-ink rounded hover:bg-raised transition-colors"
        >
          Cancel
        </button>
        <button
          @click="$emit('confirm')"
          class="px-3 py-1.5 text-sm text-white rounded transition-colors"
          :class="state.dropFirst
            ? 'bg-bad hover:bg-bad-hover'
            : 'bg-accent hover:bg-accent-hover'"
        >
          Restore
        </button>
      </div>
  </ModalShell>
</template>

<script setup>
/**
 * Confirmation for a restore, which is the one destructive thing this panel
 * does: it states what was found, what will be written, and whether the target
 * collections are dropped first.
 */
import ModalShell from './ModalShell.vue'

defineProps({
  /** `{ show, inputPath, isArchive, entries, sourceDbs, dropFirst }` */
  state: { type: Object, required: true },
  /** The connection being restored into, named in the heading. */
  connectionName: { type: String, default: 'this connection' },
})

defineEmits(['confirm', 'cancel', 'update:dropFirst'])
</script>
