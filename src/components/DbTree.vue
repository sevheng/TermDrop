<template>
  <div class="flex-1 overflow-y-auto">
    <div v-if="loading" class="flex items-center justify-center py-8">
      <Loader2 :size="16" class="animate-spin text-ink-2" />
    </div>
    <div v-else-if="databases.length === 0" class="flex flex-col items-center justify-center py-8 text-ink-3">
      <Database :size="20" class="mb-2 opacity-50" />
      <p class="text-xs">No databases loaded</p>
    </div>
    <div v-else class="py-1">
      <div
        v-for="db in databases"
        :key="db.name"
        class="border-b border-line/30"
      >
        <div
          class="w-full flex items-center gap-1.5 px-2 py-1 text-xs text-ink hover:bg-raised cursor-pointer"
          @click.self="$emit('toggle-db', db.name)"
        >
          <ChevronRight
            :size="12"
            class="transition-transform shrink-0"
            :class="expandedDbs.has(db.name) ? 'rotate-90' : ''"
            @click.stop="$emit('toggle-db', db.name)"
          />
          <input
            v-if="selectable"
            type="checkbox"
            :checked="dbSelectionState(db) === 'all'"
            :indeterminate="dbSelectionState(db) === 'some'"
            @change="$emit('toggle-db-selection', db)"
            @click.stop
            class="accent-accent shrink-0"
          />
          <Database :size="12" class="shrink-0 text-accent" />
          <span class="flex-1 truncate" @click.self="$emit('toggle-db', db.name)">{{ db.name }}</span>
          <!-- Collections load lazily, so an unexpanded database has no count to
               report; printing "0 cols" claimed it was empty. -->
          <span v-if="db.collections.length > 0" class="text-[10px] text-ink-3">
            {{ db.collections.length }} cols
          </span>
        </div>

        <div v-if="expandedDbs.has(db.name)" class="pl-6 pr-2 py-1 space-y-0.5">
          <label
            v-for="coll in db.collections"
            :key="coll"
            class="group flex items-center gap-1.5 text-[11px] text-ink hover:bg-raised px-1 py-0.5 rounded"
            :class="[
              selectable || browsable ? 'cursor-pointer' : '',
              isActive(db.name, coll) ? 'bg-selected hover:bg-selected' : '',
            ]"
            :title="browsable ? 'Click to view documents' : undefined"
          >
            <input
              v-if="selectable"
              type="checkbox"
              :checked="isSelected(db.name, coll)"
              @change="$emit('toggle-collection', db.name, coll)"
              class="accent-accent"
            />
            <Table :size="10" class="shrink-0 text-ink-3" />
            <span
              class="truncate flex-1"
              :class="isActive(db.name, coll) ? 'text-white' : ''"
              @click.stop.prevent="browsable && $emit('open-collection', db.name, coll)"
            >
              {{ coll }}
            </span>
          </label>
          <div v-if="db.collections.length === 0" class="text-[10px] text-ink-3 px-1">
            No collections
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { Database, Table, ChevronRight, Loader2 } from 'lucide-vue-next'
import { isSelected as isSelectedIn, dbSelectionState as dbSelectionStateIn } from '../utils/mongoSelection.js'

const props = defineProps({
  databases: { type: Array, required: true },
  expandedDbs: { type: Set, required: true },
  selectedCollections: { type: Map, required: true },
  selectable: { type: Boolean, default: true },
  /** Clicking a collection name opens it. */
  browsable: { type: Boolean, default: false },
  /** The collection currently open, highlighted in the tree. */
  activeDb: { type: String, default: '' },
  activeCollection: { type: String, default: '' },
  loading: { type: Boolean, default: false },
})

defineEmits(['toggle-db', 'toggle-db-selection', 'toggle-collection', 'open-collection'])

function isSelected(db, coll) {
  return isSelectedIn(props.selectedCollections, db, coll)
}

function dbSelectionState(db) {
  return dbSelectionStateIn(props.selectedCollections, db)
}

function isActive(db, coll) {
  return props.activeDb === db && props.activeCollection === coll
}
</script>
