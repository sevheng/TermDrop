<template>
  <div
    class="group flex items-center gap-1.5 py-1 px-2 rounded cursor-pointer"
    :class="isConnecting
      ? 'bg-[#0e639c]/30 opacity-60'
      : 'hover:bg-[#2a2d2e]'"
    draggable="true"
    @dragstart="onDragStart"
    @dragend="$emit('drag-end')"
    @contextmenu.prevent.stop="$emit('context-menu', $event, host)"
    @click="isConnecting || $emit('connect')"
  >
    <!-- Connection status dot -->
    <span
      class="w-1.5 h-1.5 rounded-full shrink-0"
      :class="isConnected ? 'bg-[#89d185]' : 'bg-[#6e6e6e]'"
    ></span>

    <!-- OS Icon -->
    <component :is="osIcon" :size="14" class="shrink-0 text-[#6e6e6e]" />

    <!-- Host info -->
    <div class="min-w-0 flex-1">
      <div class="text-xs text-[#cccccc] truncate">{{ host.name }}</div>
      <div class="text-[10px] text-[#858585] truncate">{{ host.username }}@{{ host.host }}:{{ host.port }}</div>
    </div>

    <!-- Actions -->
    <div class="flex items-center gap-0.5 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity">
      <button
        @click.stop="$emit('toggle-favorite')"
        class="p-0.5 text-[#6e6e6e] hover:text-[#cca700]"
        :class="host.favorite ? 'text-[#cca700] opacity-100' : ''"
        title="Toggle favorite"
      >
        <Star :size="12" :fill="host.favorite ? 'currentColor' : 'none'" />
      </button>
      <button @click.stop="$emit('edit')" class="p-0.5 text-[#6e6e6e] hover:text-[#cccccc]" title="Edit">
        <Pencil :size="12" />
      </button>
      <button @click.stop="$emit('delete')" class="p-0.5 text-[#6e6e6e] hover:text-[#f44336]" title="Delete">
        <Trash2 :size="12" />
      </button>
    </div>

    <!-- Connecting spinner -->
    <Loader2 v-if="isConnecting" :size="14" class="text-[#007acc] shrink-0 animate-spin" />
  </div>
</template>

<script setup>
import { computed } from 'vue'
import {
  Server,
  Star,
  Pencil,
  Trash2,
  Loader2,
  // OS icons
  Apple,
} from 'lucide-vue-next'

const props = defineProps({
  host: { type: Object, required: true },
  isConnected: { type: Boolean, default: false },
  isConnecting: { type: Boolean, default: false },
})

const emit = defineEmits(['connect', 'edit', 'delete', 'toggle-favorite', 'drag-start', 'drag-end', 'context-menu'])

function onDragStart(event) {
  event.dataTransfer.setData('application/json', JSON.stringify({ hostId: props.host.id }))
  event.dataTransfer.effectAllowed = 'move'

  // Compact drag ghost — mini host row
  const ghost = document.createElement('div')
  ghost.innerHTML = `<span style="opacity:0.6">${props.host.username}@${props.host.host}</span> <strong>${props.host.name}</strong>`
  ghost.style.cssText = 'padding: 2px 8px; background: #1f2937; color: #e5e7eb; border-radius: 3px; font-size: 10px; white-space: nowrap; font-family: system-ui; position: fixed; top: -9999px; pointer-events: none;'
  document.body.appendChild(ghost)
  event.dataTransfer.setDragImage(ghost, 8, 10)
  setTimeout(() => document.body.removeChild(ghost), 0)

  emit('drag-start')
}

const osIcon = computed(() => {
  const name = (props.host.name || '').toLowerCase()
  const host = (props.host.host || '').toLowerCase()
  const combined = name + ' ' + host

  if (combined.includes('mac') || combined.includes('darwin') || combined.includes('osx')) return Apple
  // Default server icon for all others (Linux, Windows, etc.)
  return Server
})
</script>
