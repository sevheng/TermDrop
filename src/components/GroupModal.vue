<template>
  <ModalShell
    @close="$emit('close')" :show="show" dim="bg-black/60" z="z-50" panel-class="p-6 w-80 shadow-xl">
      <h3 class="text-lg font-semibold text-ink mb-5">
        {{ isRename ? 'Rename Group' : 'New Group' }}
      </h3>

      <div class="space-y-4">
        <div>
          <label class="block text-xs text-ink-2 mb-1.5">
            Group Name <span class="text-bad">*</span>
          </label>
          <input
            v-model="form.name"
            ref="nameInput"
            type="text"
            :placeholder="isRename ? 'New name' : 'e.g. Production'"
            :class="inputClass('name')"
            @blur="validateField('name')"
            @keydown.enter="onSave"
          />
          <p v-if="errors.name" class="text-xs text-bad mt-1">{{ errors.name }}</p>
        </div>
      </div>

      <div class="flex justify-end gap-2 mt-6">
        <button
          @click="onClose"
          class="px-4 py-2 text-sm text-ink-2 hover:text-ink"
        >
          Cancel
        </button>
        <button
          @click="onSave"
          class="px-4 py-2 text-sm bg-accent-solid hover:bg-accent-solid-hover text-white rounded"
        >
          {{ isRename ? 'Rename' : 'Create' }}
        </button>
      </div>
  </ModalShell>
</template>

<script setup>
import { ref, watch, nextTick, computed } from 'vue'
import ModalShell from './ModalShell.vue'

const props = defineProps({
  show: Boolean,
  mode: {
    type: String,
    default: 'create', // 'create' or 'rename'
  },
  existingNames: {
    type: Array,
    default: () => [],
  },
  currentName: {
    type: String,
    default: '',
  },
})

const emit = defineEmits(['close', 'save'])

const nameInput = ref(null)
const form = ref({ name: '' })
const errors = ref({})

const isRename = computed(() => props.mode === 'rename')

watch(() => props.show, (visible) => {
  if (visible) {
    form.value.name = isRename.value ? props.currentName : ''
    errors.value = {}
    nextTick(() => nameInput.value?.focus())
  }
})

function inputClass(field) {
  const base = 'w-full bg-input border rounded px-3 py-2 text-sm text-ink focus:outline-none transition-colors'
  const error = errors.value[field] ? 'border-bad focus:border-bad' : 'border-line focus:border-accent'
  return `${base} ${error}`
}

function validateField(field) {
  const val = form.value[field]
  let msg = ''

  if (field === 'name') {
    if (!val || String(val).trim() === '') {
      msg = 'Group name is required'
    } else if (props.existingNames.includes(val.trim())) {
      msg = 'A group with this name already exists'
    } else if (isRename.value && val.trim() === props.currentName) {
      msg = 'New name must be different'
    }
  }

  if (msg) errors.value[field] = msg
  else delete errors.value[field]
}

function onClose() {
  emit('close')
}

function onSave() {
  validateField('name')
  if (Object.keys(errors.value).length > 0) return
  emit('save', form.value.name.trim())
}
</script>
