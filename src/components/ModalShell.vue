<template>
  <div
    v-if="show"
    :class="['fixed inset-0 flex items-center justify-center', dim, z]"
    @keydown="onKeydown"
  >
    <div ref="panel" role="dialog" aria-modal="true" :class="['bg-overlay rounded-lg border border-line', panelClass]">
      <slot />
    </div>
  </div>
</template>

<script setup>
/**
 * Full-screen dimmed overlay with a centered panel.
 *
 * Also the app's focus trap. There was none, so Tab walked out of every
 * dialog and into the application behind the scrim, where the controls are
 * covered and cannot be seen. Eleven modals use this, so one fix serves all
 * of them: focus enters on open, Tab cycles inside, Escape closes, and focus
 * returns to whatever opened it.
 */
import { ref, watch, nextTick, onBeforeUnmount } from 'vue'
import { FOCUSABLE, visibleOnly, nextFocusIndex } from '../utils/focusTrap.js'

const props = defineProps({
  show: { type: Boolean, default: false },
  dim: { type: String, default: 'bg-black/60' },
  z: { type: String, default: 'z-50' },
  panelClass: { type: String, default: 'p-6 w-[28rem] shadow-xl' },
  /** Some dialogs are dismissed by their own buttons only. */
  closeOnEscape: { type: Boolean, default: true },
})

const emit = defineEmits(['close'])

const panel = ref(null)
let restoreTo = null

function focusables() {
  return visibleOnly(panel.value?.querySelectorAll(FOCUSABLE))
}

function onKeydown(e) {
  if (e.key === 'Escape' && props.closeOnEscape) {
    // Stopped here so a nested menu's Escape does not also reach the dialog
    // behind this one.
    e.stopPropagation()
    emit('close')
    return
  }
  if (e.key !== 'Tab') return

  const items = focusables()
  if (items.length === 0) {
    e.preventDefault()
    return
  }
  const from = items.indexOf(document.activeElement)
  const next = nextFocusIndex(items.length, from, e.shiftKey ? -1 : 1)
  e.preventDefault()
  items[next]?.focus()
}

watch(
  () => props.show,
  async open => {
    if (open) {
      restoreTo = document.activeElement
      await nextTick()
      // The first field, or the panel itself, so the trap has somewhere to
      // start from and a screen reader announces the dialog.
      const items = focusables()
      ;(items[0] || panel.value)?.focus?.()
    } else if (restoreTo?.focus) {
      restoreTo.focus()
      restoreTo = null
    }
  },
)

onBeforeUnmount(() => {
  restoreTo = null
})
</script>
