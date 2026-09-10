<template>
  <ModalShell
    @close="$emit('cancel')" :show="show" dim="bg-black/60" z="z-[100]" panel-class="p-5 w-80 shadow-xl">
      <h3 class="text-base font-semibold text-ink mb-1">{{ title }}</h3>
      <p v-if="message" class="text-sm text-ink-2 mb-3">{{ message }}</p>

      <input
        ref="inputRef"
        v-model="inputValue"
        :type="type"
        :placeholder="placeholder"
        class="w-full bg-input border border-line rounded px-3 py-2 text-sm text-ink placeholder-ink-3 focus:outline-none focus:border-accent mb-4"
        @keydown.enter="onConfirm"
        @keydown.esc="onCancel"
      />

      <div class="flex justify-end gap-2">
        <button
          @click="onCancel"
          class="px-3 py-1.5 text-sm text-ink-2 hover:text-ink rounded hover:bg-raised transition-colors"
        >
          {{ cancelText }}
        </button>
        <button
          @click="onConfirm"
          :disabled="!inputValue.trim()"
          :class="[
            'px-3 py-1.5 text-sm text-white rounded transition-colors',
            danger
              ? 'bg-bad-solid hover:bg-bad-solid-hover disabled:bg-bad-solid/30'
              : 'bg-accent-solid hover:bg-accent-solid-hover disabled:bg-accent-solid/30'
          ]"
        >
          {{ confirmText }}
        </button>
      </div>
  </ModalShell>
</template>

<script setup>
import { ref, watch, nextTick } from 'vue'
import ModalShell from './ModalShell.vue'

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
