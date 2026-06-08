<template>
  <div
    v-if="show"
    class="fixed inset-0 bg-black/60 flex items-center justify-center z-50"
    @click.self="onClose"
  >
    <div class="bg-[#252526] rounded-lg p-6 w-80 border border-[#3c3c3c] shadow-xl">
      <h3 class="text-lg font-semibold text-[#cccccc] mb-5">
        {{ isRename ? 'Rename Group' : 'New Group' }}
      </h3>

      <div class="space-y-4">
        <div>
          <label class="block text-xs text-[#858585] mb-1.5">
            Group Name <span class="text-[#f44336]">*</span>
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
          <p v-if="errors.name" class="text-xs text-[#f44336] mt-1">{{ errors.name }}</p>
        </div>
      </div>

      <div class="flex justify-end gap-2 mt-6">
        <button
          @click="onClose"
          class="px-4 py-2 text-sm text-[#858585] hover:text-[#cccccc]"
        >
          Cancel
        </button>
        <button
          @click="onSave"
          class="px-4 py-2 text-sm bg-[#0e639c] hover:bg-[#1177bb] text-white rounded"
        >
          {{ isRename ? 'Rename' : 'Create' }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, watch, nextTick, computed } from 'vue'

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
  const base = 'w-full bg-[#3c3c3c] border rounded px-3 py-2 text-sm text-[#cccccc] focus:outline-none transition-colors'
  const error = errors.value[field] ? 'border-[#f44336] focus:border-[#f44336]' : 'border-[#3c3c3c] focus:border-[#007acc]'
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
