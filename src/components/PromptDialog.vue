<template>
  <div
    v-if="show"
    class="fixed inset-0 bg-black/60 flex items-center justify-center z-[100]"
  >
    <div class="bg-[#252526] rounded-lg p-5 w-80 border border-[#3c3c3c] shadow-xl">
      <h3 class="text-base font-semibold text-[#cccccc] mb-1">{{ title }}</h3>
      <p v-if="message" class="text-sm text-[#858585] mb-3">{{ message }}</p>

      <input
        ref="inputRef"
        v-model="inputValue"
        :type="type"
        :placeholder="placeholder"
        class="w-full bg-[#3c3c3c] border border-[#3c3c3c] rounded px-3 py-2 text-sm text-[#cccccc] placeholder-[#6e6e6e] focus:outline-none focus:border-[#007acc] mb-4"
        @keydown.enter="onConfirm"
        @keydown.esc="onCancel"
      />

      <div class="flex justify-end gap-2">
        <button
          @click="onCancel"
          class="px-3 py-1.5 text-sm text-[#858585] hover:text-[#cccccc] rounded hover:bg-[#2a2d2e] transition-colors"
        >
          {{ cancelText }}
        </button>
        <button
          @click="onConfirm"
          :disabled="!inputValue.trim()"
          :class="[
            'px-3 py-1.5 text-sm text-white rounded transition-colors',
            danger
              ? 'bg-[#f44336] hover:bg-[#d32f2f] disabled:bg-[#f44336]/30'
              : 'bg-[#0e639c] hover:bg-[#1177bb] disabled:bg-[#0e639c]/30'
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
  type: { type: String, default: 'text' },
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
