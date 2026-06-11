<template>
  <div class="flex-1 overflow-y-auto">
    <div v-if="loading" class="flex items-center justify-center py-8">
      <Loader2 :size="16" class="animate-spin text-[#858585]" />
    </div>
    <div v-else-if="databases.length === 0" class="flex flex-col items-center justify-center py-8 text-[#6e6e6e]">
      <Database :size="20" class="mb-2 opacity-50" />
      <p class="text-xs">No databases loaded</p>
    </div>
    <div v-else class="py-1">
      <div
        v-for="db in databases"
        :key="db.name"
        class="border-b border-[#3c3c3c]/30"
      >
        <div
          class="w-full flex items-center gap-1.5 px-2 py-1 text-xs text-[#cccccc] hover:bg-[#2a2d2e] cursor-pointer"
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
            class="accent-[#007acc] shrink-0"
          />
          <Database :size="12" class="shrink-0 text-[#007acc]" />
          <span class="flex-1 truncate" @click.self="$emit('toggle-db', db.name)">{{ db.name }}</span>
          <span class="text-[10px] text-[#6e6e6e]">{{ db.collections.length }} cols</span>
        </div>

        <div v-if="expandedDbs.has(db.name)" class="pl-6 pr-2 py-1 space-y-0.5">
          <label
            v-for="coll in db.collections"
            :key="coll"
            class="flex items-center gap-1.5 text-[11px] text-[#cccccc] hover:bg-[#2a2d2e] px-1 py-0.5 rounded"
            :class="selectable ? 'cursor-pointer' : ''"
          >
            <input
              v-if="selectable"
              type="checkbox"
              :checked="isSelected(db.name, coll)"
              @change="$emit('toggle-collection', db.name, coll)"
              class="accent-[#007acc]"
            />
            <Table :size="10" class="shrink-0 text-[#6e6e6e]" />
            <span class="truncate">{{ coll }}</span>
          </label>
          <div v-if="db.collections.length === 0" class="text-[10px] text-[#6e6e6e] px-1">
            No collections
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { Database, Table, ChevronRight, Loader2 } from 'lucide-vue-next'

const props = defineProps({
  databases: { type: Array, required: true },
  expandedDbs: { type: Set, required: true },
  selectedCollections: { type: Map, required: true },
  selectable: { type: Boolean, default: true },
  loading: { type: Boolean, default: false },
})

defineEmits(['toggle-db', 'toggle-db-selection', 'toggle-collection'])

function isSelected(db, coll) {
  return props.selectedCollections.get(db)?.has(coll) || false
}

function dbSelectionState(db) {
  const selected = props.selectedCollections.get(db.name)
  if (!selected || selected.size === 0) return 'none'
  if (db.collections.length > 0 && selected.size === db.collections.length) return 'all'
  return 'some'
}
</script>
