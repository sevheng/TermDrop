<template>
  <ModalShell :show="show" dim="bg-black/60" z="z-[100]" panel-class="p-5 w-80 shadow-xl">
      <div class="flex items-start gap-3 mb-4">
        <div class="shrink-0 mt-0.5">
          <AlertTriangle :size="20" class="text-warn" />
        </div>
        <div>
          <h3 class="text-base font-semibold text-ink">{{ title }}</h3>
          <p class="text-sm text-ink-2 mt-1">{{ message }}</p>
        </div>
      </div>

      <div class="flex justify-end gap-2">
        <button
          @click="onCancel"
          class="px-3 py-1.5 text-sm text-ink-2 hover:text-ink rounded hover:bg-raised transition-colors"
        >
          {{ cancelText }}
        </button>
        <button
          @click="onConfirm"
          :class="[
            'px-3 py-1.5 text-sm text-white rounded transition-colors',
            danger
              ? 'bg-bad-solid hover:bg-bad-solid-hover'
              : 'bg-accent-solid hover:bg-accent-solid-hover'
          ]"
        >
          {{ confirmText }}
        </button>
      </div>
  </ModalShell>
</template>

<script setup>
import { AlertTriangle } from 'lucide-vue-next'
import ModalShell from './ModalShell.vue'

defineProps({
  show: Boolean,
  title: { type: String, default: 'Confirm' },
  message: { type: String, required: true },
  confirmText: { type: String, default: 'Confirm' },
  cancelText: { type: String, default: 'Cancel' },
  danger: { type: Boolean, default: false },
})

const emit = defineEmits(['confirm', 'cancel'])

function onConfirm() {
  emit('confirm')
}

function onCancel() {
  emit('cancel')
}
</script>
