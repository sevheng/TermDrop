<template>
  <div
    class="flex flex-col items-center justify-center text-center px-4"
    :class="size === 'md' ? 'py-16' : 'py-10'"
    :aria-busy="state === 'loading' || undefined"
  >
    <Loader2
      v-if="state === 'loading'"
      :size="iconSize"
      class="mb-2 text-ink-3 animate-spin"
    />
    <component
      v-else
      :is="resolvedIcon"
      :size="iconSize"
      class="mb-2 opacity-50"
      :class="state === 'error' ? 'text-bad' : 'text-ink-3'"
    />

    <p class="text-xs" :class="state === 'error' ? 'text-bad' : 'text-ink'">
      {{ title || defaultTitle }}
    </p>

    <p v-if="hint || $slots.hint" class="text-2xs text-ink-2 mt-1 max-w-xs">
      <slot name="hint">{{ hint }}</slot>
    </p>

    <slot />

    <button
      v-if="actionLabel"
      type="button"
      class="mt-3 inline-flex items-center gap-1.5 rounded px-3 py-1 text-xs
             bg-accent-solid hover:bg-accent-solid-hover text-white
             disabled:opacity-40 disabled:pointer-events-none"
      :disabled="actionPending"
      @click="$emit('action')"
    >
      <Loader2 v-if="actionPending" :size="12" class="animate-spin" />
      <component v-else-if="actionIcon" :is="actionIcon" :size="12" />
      {{ actionPending ? pendingLabel || 'Working…' : actionLabel }}
    </button>

    <slot name="action" />
  </div>
</template>

<script setup>
/**
 * The one shape for "there is nothing to show here".
 *
 * There were nineteen of these, across five icon sizes, four text sizes and
 * five vertical treatments. Only two offered an action, and only four told
 * "nothing here yet" apart from "nothing matched your filter" — which is the
 * distinction that actually decides what the reader should do next.
 *
 * `state` is required for exactly that reason: a component cannot render an
 * empty state without having decided which one it is.
 */
import { computed } from 'vue'
import { Loader2, Inbox, SearchX, AlertCircle } from 'lucide-vue-next'

const props = defineProps({
  /** 'loading' | 'empty' | 'filtered' | 'error' */
  state: {
    type: String,
    required: true,
    validator: v => ['loading', 'empty', 'filtered', 'error'].includes(v),
  },
  icon: { type: [Object, Function], default: null },
  title: { type: String, default: '' },
  hint: { type: String, default: '' },
  actionLabel: { type: String, default: '' },
  actionIcon: { type: [Object, Function], default: null },
  actionPending: { type: Boolean, default: false },
  pendingLabel: { type: String, default: '' },
  /** 'sm' for the 288px side panels, 'md' for a full tab body. */
  size: { type: String, default: 'sm' },
})

defineEmits(['action'])

const iconSize = computed(() => (props.size === 'md' ? 32 : 24))

const DEFAULT_ICONS = { empty: Inbox, filtered: SearchX, error: AlertCircle }
const resolvedIcon = computed(() => props.icon || DEFAULT_ICONS[props.state] || Inbox)

const DEFAULT_TITLES = {
  loading: 'Loading…',
  empty: 'Nothing here yet',
  filtered: 'Nothing matches',
  error: 'Something went wrong',
}
const defaultTitle = computed(() => DEFAULT_TITLES[props.state] || '')
</script>
