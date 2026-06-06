<template>
  <div
    v-if="show"
    class="fixed inset-0 bg-black/60 flex items-center justify-center z-[100]"
    @click.self="onCancel"
  >
    <div class="bg-gray-800 rounded-lg p-5 w-80 border border-gray-700 shadow-xl">
      <h3 class="text-base font-semibold text-white mb-1">{{ title }}</h3>
      <p v-if="message" class="text-sm text-gray-300 mb-3">{{ message }}</p>

      <input
        ref="inputRef"
        v-model="inputValue"
        type="text"
        :placeholder="placeholder"
        class="w-full bg-gray-700 border border-gray-600 rounded px-3 py-2 text-sm text-white placeholder-gray-500 focus:outline-none focus:border-blue-500 mb-4"
        @keydown.enter="onConfirm"
        @keydown.esc="onCancel"
      />

      <div class="flex justify-end gap-2">
        <button
          @click="onCancel"
          class="px-3 py-1.5 text-sm text-gray-300 hover:text-white rounded hover:bg-gray-700 transition-colors"
        >
          {{ cancelText }}
        </button>
        <button
          @click="onConfirm"
          :disabled="!inputValue.trim()"
          :class="[
            'px-3 py-1.5 text-sm text-white rounded transition-colors',
            danger
              ? 'bg-red-600 hover:bg-red-700 disabled:bg-red-900/50'
              : 'bg-blue-600 hover:bg-blue-700 disabled:bg-blue-900/50'
          ]"
        >
          {{ confirmText }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, watch, nextTick } from 'vue'

const props = defineProps({
  show: Boolean,
  title: { type: String, default: 'Prompt' },
  message: { type: String, default: '' },
  placeholder: { type: String, default: '' },
  defaultValue: { type: String, default: '' },
  confirmText: { type: String, default: 'Confirm' },
  cancelText: { type: String, default: 'Cancel' },
  danger: { type: Boolean, default: false },
})

const emit = defineEmits(['confirm', 'cancel'])

const inputRef = ref(null)
const inputValue = ref('')

watch(() => props.show, (isOpen) => {
  if (isOpen) {
    inputValue.value = props.defaultValue
    nextTick(() => {
      inputRef.value?.focus()
      if (props.defaultValue) {
        inputRef.value?.select()
      }
    })
  }
})

function onConfirm() {
  const value = inputValue.value.trim()
  if (!value) return
  emit('confirm', value)
}

function onCancel() {
  emit('cancel')
}
</script>
