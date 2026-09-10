<template>
  <div
    class="group relative flex items-center gap-2 h-8 pl-2 pr-1 rounded-sm cursor-pointer"
    :class="[
      state.active ? 'bg-selected' : 'hover:bg-raised',
      state.connecting ? 'opacity-60' : '',
    ]"
    draggable="true"
    @dragstart="onDragStart"
    @dragend="$emit('drag-end')"
    @contextmenu.prevent.stop="$emit('context-menu', $event, host)"
    @click="state.connecting || $emit('connect')"
  >
    <!--
      A rail rather than a dot. It reads at the row's edge, never competes with
      the icon, and there is room for it to say something a 6px dot cannot.
    -->
    <span
      v-if="state.connected"
      class="absolute left-0 inset-y-1 w-0.5 rounded-full bg-good"
      :title="state.tabCount > 1 ? `${state.tabCount} open tabs` : 'Connected'"
    ></span>

    <component :is="rowIcon" :size="14" class="shrink-0" :class="hostKindIconClass(host)" />

    <div class="min-w-0 flex-1">
      <div class="text-xs text-ink truncate leading-tight">{{ host.name }}</div>
      <div class="text-2xs text-ink-2 truncate leading-tight" :title="subtitle">{{ subtitle }}</div>
    </div>

    <!-- Shown at rest. It used to live inside the hover cluster with its own
         opacity-100, which a parent's opacity-0 makes impossible, so a
         favourited host displayed no star at all. -->
    <Star
      v-if="host.favorite"
      :size="11"
      class="shrink-0 text-warn group-hover:opacity-0 transition-opacity"
      fill="currentColor"
    />

    <span v-if="age && !state.connected" class="shrink-0 text-2xs text-ink-3 tabular-nums group-hover:opacity-0 transition-opacity">
      {{ age }}
    </span>

    <!--
      Absolutely positioned so revealing them never reflows the row or steals
      width from the name, with a fade so they do not sit on top of the text.
    -->
    <div
      class="absolute right-1 inset-y-0 hidden items-center gap-0.5 pl-6 group-hover:flex group-focus-within:flex"
      :class="state.active ? 'bg-gradient-to-l from-selected via-selected to-transparent'
                           : 'bg-gradient-to-l from-raised via-raised to-transparent'"
    >
      <IconButton
        :icon="Star"
        :label="host.favorite ? 'Remove from favourites' : 'Add to favourites'"
        tone="warn"
        :active="!!host.favorite"
        @click.stop="$emit('toggle-favorite')"
      />
      <IconButton :icon="Pencil" label="Edit host" @click.stop="$emit('edit')" />
      <IconButton :icon="Trash2" label="Delete host" tone="bad" @click.stop="$emit('delete')" />
    </div>

    <Loader2 v-if="state.connecting" :size="12" class="shrink-0 text-accent animate-spin" />
  </div>
</template>

<script setup>
/**
 * One host in the sidebar.
 *
 * Two lines, because several hosts can share a name and the address line is
 * what tells them apart. The height is declared (h-8) rather than emergent:
 * it used to come out at ~39px, most of the excess from a font size set with
 * no matching line-height.
 */
import { computed } from 'vue'
import { Star, Pencil, Trash2, Loader2, Server, Database, Layers, Apple } from 'lucide-vue-next'
import IconButton from './IconButton.vue'
import { mongoDisplayUri } from '../utils/mongoDisplay.js'
import { redisDisplayUri } from '../utils/redisUri.js'
import { hostKind, HOST_KIND, hostKindIconClass } from '../utils/hostKind.js'
import { formatAge } from '../utils/hostRowState.js'

const props = defineProps({
  host: { type: Object, required: true },
  /** From `hostRowState()` — connected, active, connecting and tabCount. */
  state: {
    type: Object,
    default: () => ({ connected: false, active: false, connecting: false, tabCount: 0 }),
  },
})

const emit = defineEmits([
  'connect', 'edit', 'delete', 'toggle-favorite', 'drag-start', 'drag-end', 'context-menu',
])

const kind = computed(() => hostKind(props.host))

/** What the row is, for the drag ghost. */
const kindLabel = computed(
  () =>
    ({ [HOST_KIND.MONGODB]: 'MongoDB', [HOST_KIND.REDIS]: 'Redis' })[kind.value] ||
    `${props.host.username}@${props.host.host}`,
)

/** Written on every SSH connect and, until now, read nowhere in the app. */
const age = computed(() => formatAge(props.host.last_connected_at))

function onDragStart(event) {
  event.dataTransfer.setData('application/json', JSON.stringify({ hostId: props.host.id }))
  event.dataTransfer.effectAllowed = 'move'

  const ghost = document.createElement('div')
  ghost.innerHTML = `<span style="opacity:0.6">${kindLabel.value}</span> <strong>${props.host.name}</strong>`
  ghost.style.cssText =
    'padding: 2px 8px; background: rgb(var(--td-overlay)); color: rgb(var(--td-ink)); border-radius: 3px; font-size: 10px; white-space: nowrap; font-family: system-ui; position: fixed; top: -9999px; pointer-events: none;'
  document.body.appendChild(ghost)
  event.dataTransfer.setDragImage(ghost, 8, 10)
  setTimeout(() => document.body.removeChild(ghost), 0)

  emit('drag-start')
}

const rowIcon = computed(() => {
  if (kind.value === HOST_KIND.MONGODB) return Database
  if (kind.value === HOST_KIND.REDIS) return Layers

  const combined = `${props.host.name || ''} ${props.host.host || ''}`.toLowerCase()
  if (/\b(mac|darwin|osx)\b/.test(combined)) return Apple
  return Server
})

const subtitle = computed(() => {
  if (kind.value === HOST_KIND.MONGODB) return mongoDisplayUri(props.host.mongo_uri)
  if (kind.value === HOST_KIND.REDIS) {
    const via = props.host.redis_tunnel_host_id ? ' · tunnelled' : ''
    return redisDisplayUri(props.host.redis_uri) + via
  }
  return `${props.host.username}@${props.host.host}:${props.host.port}`
})
</script>
