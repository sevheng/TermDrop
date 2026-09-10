<template>
  <div v-if="show" :class="['fixed inset-0 flex items-center justify-center', dim, z]">
    <div :class="['bg-overlay rounded-lg border border-line', panelClass]">
      <slot />
    </div>
  </div>
</template>

<script setup>
/**
 * Full-screen dimmed overlay with a centered panel. `dim` and `z` set the
 * overlay's backdrop and stacking classes; `panelClass` carries the
 * panel's size, padding, and shadow classes.
 */
defineProps({
  show: { type: Boolean, default: false },
  dim: { type: String, default: 'bg-black/60' },
  z: { type: String, default: 'z-50' },
  panelClass: { type: String, default: 'p-6 w-[28rem] shadow-xl' },
})
</script>
