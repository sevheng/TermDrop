<template>
  <div class="flex-1 overflow-y-auto">
    <EmptyState v-if="loading" state="loading" title="Loading databases…" />
    <EmptyState
      v-else-if="databases.length === 0"
      state="empty"
      :icon="Layers"
      title="No keys on this server"
      hint="Every database is empty"
    />
    <div v-else class="py-1">
      <div v-for="dbInfo in databases" :key="dbInfo.index" class="border-b border-line/30">
        <div
          class="w-full flex items-center gap-1.5 px-2 py-1 text-xs cursor-pointer hover:bg-raised"
          :class="dbInfo.index === activeDb ? 'bg-active text-ink' : 'text-ink'"
          @click="$emit('select-db', dbInfo.index)"
        >
          <ChevronRight
            :size="12"
            class="transition-transform shrink-0"
            :class="dbInfo.index === activeDb ? 'rotate-90' : ''"
          />
          <Layers :size="12" class="shrink-0 text-redis" />
          <span class="flex-1 truncate">db{{ dbInfo.index }}</span>
          <span class="text-2xs text-ink-3">{{ dbInfo.keys.toLocaleString() }}</span>
        </div>

        <div v-if="dbInfo.index === activeDb" class="pb-1">
          <div v-if="treeLoading" class="flex items-center gap-2 px-6 py-1.5 text-xs text-ink-2">
            <Loader2 :size="11" class="animate-spin" />
            Grouping keys…
          </div>
          <template v-else>
            <button
              v-for="group in groups"
              :key="group.prefix"
              class="w-full flex items-center gap-1.5 pl-7 pr-2 py-1 text-xs text-left hover:bg-raised"
              :class="group.prefix === activeGroup ? 'text-ink bg-raised' : 'text-ink-2'"
              @click="$emit('select-group', group)"
            >
              <span class="flex-1 truncate" :title="group.binary ? 'binary prefix' : group.prefix">
                {{ group.binary ? '⟨binary⟩' : group.prefix }}
              </span>
              <span class="text-2xs text-ink-3">{{ group.count.toLocaleString() }}</span>
            </button>

            <!--
              The scan is capped, so these groups may describe only part of the
              keyspace. Saying so is the whole point: a tree that implied it was
              complete would be quietly wrong on any real server.
            -->
            <p v-if="truncated" class="pl-7 pr-2 py-1 text-2xs text-ink-3 italic">
              ⋯ partial: grouped the first {{ scanned.toLocaleString() }} keys
            </p>
            <p
              v-else-if="groups.length === 0"
              class="pl-7 pr-2 py-1 text-2xs text-ink-3 italic"
            >
              No keys match
            </p>
          </template>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
/**
 * The keyspace tree: databases, and the prefix groups of the selected one.
 *
 * Fully controlled, like `DbTree`. The parent owns the scan and the selection;
 * this only draws them.
 */
import { ChevronRight, Layers, Loader2 } from 'lucide-vue-next'
import EmptyState from './EmptyState.vue'

defineProps({
  databases: { type: Array, required: true },
  activeDb: { type: Number, default: null },
  groups: { type: Array, default: () => [] },
  activeGroup: { type: String, default: null },
  scanned: { type: Number, default: 0 },
  truncated: { type: Boolean, default: false },
  loading: { type: Boolean, default: false },
  treeLoading: { type: Boolean, default: false },
})

defineEmits(['select-db', 'select-group'])
</script>
