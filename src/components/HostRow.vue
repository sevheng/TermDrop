<template>
  <div
    class="group flex items-center gap-1.5 py-1 px-2 rounded cursor-pointer"
    :class="isConnecting
      ? 'bg-accent-solid/30 opacity-60'
      : 'hover:bg-raised'"
    draggable="true"
    @dragstart="onDragStart"
    @dragend="$emit('drag-end')"
    @contextmenu.prevent.stop="$emit('context-menu', $event, host)"
    @click="isConnecting || $emit('connect')"
  >
    <!-- Connection status dot -->
    <span
      class="w-1.5 h-1.5 rounded-full shrink-0"
      :class="isConnected ? 'bg-good' : 'bg-ink-3'"
    ></span>

    <!-- Icon: Database for MongoDB-only, OS icon for SSH -->
    <component :is="rowIcon" :size="14" class="shrink-0 text-ink-3" />

    <!-- Host info -->
    <div class="min-w-0 flex-1">
      <div class="text-xs text-ink truncate">{{ host.name }}</div>
      <div class="text-2xs text-ink-2 truncate">{{ subtitle }}</div>
    </div>

    <!-- Actions -->
    <div class="flex items-center gap-0.5 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity">
      <button
        @click.stop="$emit('toggle-favorite')"
        class="p-0.5 text-ink-3 hover:text-warn"
        :class="host.favorite ? 'text-warn opacity-100' : ''"
        title="Toggle favorite"
      >
        <Star :size="12" :fill="host.favorite ? 'currentColor' : 'none'" />
      </button>
      <button @click.stop="$emit('edit')" class="p-0.5 text-ink-3 hover:text-ink" title="Edit">
        <Pencil :size="12" />
      </button>
      <button @click.stop="$emit('delete')" class="p-0.5 text-ink-3 hover:text-bad" title="Delete">
        <Trash2 :size="12" />
      </button>
    </div>

    <!-- Connecting spinner -->
    <Loader2 v-if="isConnecting" :size="14" class="text-accent shrink-0 animate-spin" />
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { mongoDisplayUri } from '../utils/mongoDisplay.js'
import { redisDisplayUri } from '../utils/redisUri.js'
import { hostKind, HOST_KIND } from '../utils/hostKind.js'
import {
  Server,
  Star,
  Pencil,
  Trash2,
  Loader2,
  Database,
  Layers,
  // OS icons
  Apple,
} from 'lucide-vue-next'

const props = defineProps({
  host: { type: Object, required: true },
  isConnected: { type: Boolean, default: false },
  isConnecting: { type: Boolean, default: false },
})

const emit = defineEmits(['connect', 'edit', 'delete', 'toggle-favorite', 'drag-start', 'drag-end', 'context-menu'])

const kind = computed(() => hostKind(props.host))
const isMongoOnly = computed(() => kind.value === HOST_KIND.MONGODB)
const isRedisOnly = computed(() => kind.value === HOST_KIND.REDIS)

/** What the row is, for the drag ghost and the accessible label. */
const kindLabel = computed(
  () =>
    ({ [HOST_KIND.MONGODB]: 'MongoDB', [HOST_KIND.REDIS]: 'Redis' })[kind.value] ||
    `${props.host.username}@${props.host.host}`,
)

function onDragStart(event) {
  event.dataTransfer.setData('application/json', JSON.stringify({ hostId: props.host.id }))
  event.dataTransfer.effectAllowed = 'move'

  // Compact drag ghost — mini host row
  const ghost = document.createElement('div')
  ghost.innerHTML = `<span style="opacity:0.6">${kindLabel.value}</span> <strong>${props.host.name}</strong>`
  ghost.style.cssText = 'padding: 2px 8px; background: rgb(var(--td-overlay)); color: rgb(var(--td-ink)); border-radius: 3px; font-size: 10px; white-space: nowrap; font-family: system-ui; position: fixed; top: -9999px; pointer-events: none;'
  document.body.appendChild(ghost)
  event.dataTransfer.setDragImage(ghost, 8, 10)
  setTimeout(() => document.body.removeChild(ghost), 0)

  emit('drag-start')
}

const rowIcon = computed(() => {
  if (isMongoOnly.value) return Database
  if (isRedisOnly.value) return Layers

  const name = (props.host.name || '').toLowerCase()
  const host = (props.host.host || '').toLowerCase()
  const combined = name + ' ' + host

  if (combined.includes('mac') || combined.includes('darwin') || combined.includes('osx')) return Apple
  // Default server icon for all others (Linux, Windows, etc.)
  return Server
})

const subtitle = computed(() => {
  if (isMongoOnly.value) return mongoDisplayUri(props.host.mongo_uri)
  if (isRedisOnly.value) {
    // Naming the bastion matters: two rows can carry the same private address
    // and mean different machines.
    const via = props.host.redis_tunnel_host_id ? ' · tunnelled' : ''
    return redisDisplayUri(props.host.redis_uri) + via
  }
  return `${props.host.username}@${props.host.host}:${props.host.port}`
})
</script>
