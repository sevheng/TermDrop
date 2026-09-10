<template>
  <button
    type="button"
    :title="label"
    :aria-label="label"
    :aria-pressed="active === null ? undefined : active"
    :aria-busy="pending || undefined"
    :disabled="disabled || pending"
    :class="[
      'inline-flex items-center justify-center rounded shrink-0 transition-colors',
      'text-ink-2 hover:bg-raised',
      TONE_HOVER[tone] || 'hover:text-ink',
      size === 'md' ? 'h-7 w-7' : 'h-6 w-6',
      active ? 'bg-active text-ink' : '',
      'focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-accent',
      'disabled:opacity-40 disabled:pointer-events-none',
    ]"
    @click="$emit('click', $event)"
  >
    <Loader2 v-if="pending" :size="glyph" class="animate-spin" />
    <component v-else :is="icon" :size="glyph" />
  </button>
</template>

<script setup>
/**
 * An icon-only control that is still labelled and still hittable.
 *
 * Two things this fixes. The label was optional and usually a bare `title`,
 * so screen readers got nothing; here it is required and becomes both `title`
 * and `aria-label`. And the old buttons were `p-0.5` around a 12px glyph — a
 * 16px target, well under the ~24px floor for a pointer — with no background
 * on hover, so nothing told you where the target was.
 */
import { computed } from 'vue'
import { Loader2 } from 'lucide-vue-next'

const props = defineProps({
  icon: { type: [Object, Function], required: true },
  /** Required: there is no unlabelled icon button. */
  label: { type: String, required: true },
  size: { type: String, default: 'sm' },
  /** Tints the hover state: 'good' | 'bad' | 'warn' | 'accent'. */
  tone: { type: String, default: 'default' },
  disabled: { type: Boolean, default: false },
  pending: { type: Boolean, default: false },
  /** Set for a toggle; drives aria-pressed and a persistent background. */
  active: { type: Boolean, default: null },
})

defineEmits(['click'])

const TONE_HOVER = {
  default: 'hover:text-ink',
  good: 'hover:text-good',
  bad: 'hover:text-bad',
  warn: 'hover:text-warn',
  accent: 'hover:text-accent',
}

const glyph = computed(() => (props.size === 'md' ? 14 : 12))
</script>
