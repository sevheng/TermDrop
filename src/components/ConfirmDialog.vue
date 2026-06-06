<template>
  <div
    v-if="show"
    class="fixed inset-0 bg-black/60 flex items-center justify-center z-[100]"
    @click.self="onCancel"
  >
    <div class="bg-gray-800 rounded-lg p-5 w-80 border border-gray-700 shadow-xl">
      <div class="flex items-start gap-3 mb-4">
        <div class="shrink-0 mt-0.5">
          <AlertTriangle :size="20" class="text-yellow-400" />
        </div>
        <div>
          <h3 class="text-base font-semibold text-white">{{ title }}</h3>
          <p class="text-sm text-gray-300 mt-1">{{ message }}</p>
        </div>
      </div>

      <div class="flex justify-end gap-2">
        <button
          @click="onCancel"
          class="px-3 py-1.5 text-sm text-gray-300 hover:text-white rounded hover:bg-gray-700 transition-colors"
        >
          {{ cancelText }}
        </button>
        <button
          @click="onConfirm"
          :class="[
            'px-3 py-1.5 text-sm text-white rounded transition-colors',
            danger
              ? 'bg-red-600 hover:bg-red-700'
              : 'bg-blue-600 hover:bg-blue-700'
          ]"
        >
          {{ confirmText }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { AlertTriangle } from 'lucide-vue-next'

const props = defineProps({
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
