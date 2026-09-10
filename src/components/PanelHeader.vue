<template>
  <div
    class="flex items-center gap-2 shrink-0 border-b border-line bg-surface"
    :class="dense ? 'h-7 px-2' : 'h-9 px-3'"
  >
    <component v-if="icon" :is="icon" :size="dense ? 12 : 14" class="shrink-0" :class="iconClass" />

    <span class="text-xs font-medium text-ink truncate shrink-0">{{ title }}</span>

    <span
      v-if="subtitle"
      class="text-2xs text-ink-3 truncate min-w-0"
      :class="subtitleMono ? 'font-mono' : ''"
      :title="subtitleTitle || subtitle"
    >{{ subtitle }}</span>

    <slot name="badges" />

    <span class="flex-1 min-w-0" />

    <span v-if="meta" class="text-2xs text-ink-3 tabular-nums shrink-0">{{ meta }}</span>
    <slot name="actions" />
  </div>
</template>

<script setup>
/**
 * The strip at the top of a panel.
 *
 * Five panels drew this five different ways — different heights, gutters,
 * type sizes and whether the strip was tinted at all. A tinted strip on an
 * untinted body is what makes a panel read as a panel, so it is tinted here
 * for all of them.
 */
defineProps({
  title: { type: String, required: true },
  subtitle: { type: String, default: '' },
  /** Full text for a subtitle that truncates. */
  subtitleTitle: { type: String, default: '' },
  subtitleMono: { type: Boolean, default: false },
  icon: { type: [Object, Function], default: null },
  /** e.g. 'text-redis' — brand colour for the datastore panels. */
  iconClass: { type: String, default: 'text-ink-3' },
  /** Right-aligned count or status, tabular so it does not jitter. */
  meta: { type: [String, Number], default: '' },
  /** Compact height for the 288px side panels. */
  dense: { type: Boolean, default: false },
})
</script>
