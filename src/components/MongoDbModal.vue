<template>
  <div
    v-if="show"
    class="fixed inset-0 bg-black/60 flex items-center justify-center z-50"
    @click.self="onClose"
  >
    <div class="bg-[#252526] rounded-lg p-6 w-[28rem] border border-[#3c3c3c] shadow-xl">
      <h3 class="text-lg font-semibold text-[#cccccc] mb-5">
        {{ isEditing ? 'Edit MongoDB' : 'Add MongoDB' }}
      </h3>

      <div class="space-y-4">
        <!-- Name -->
        <div>
          <label class="block text-xs text-[#858585] mb-1.5">Name <span class="text-[#f44336]">*</span></label>
          <input
            v-model="form.name"
            type="text"
            placeholder="Staging DB"
            class="w-full bg-[#3c3c3c] border rounded px-3 py-2 text-sm text-[#cccccc] focus:outline-none transition-colors border-[#3c3c3c] focus:border-[#007acc]"
          />
        </div>

        <!-- Remote URI -->
        <div>
          <label class="block text-xs text-[#858585] mb-1.5">Remote URI <span class="text-[#f44336]">*</span></label>
          <input
            v-model="form.mongo_uri"
            type="text"
            placeholder="mongodb://user:pass@host:27017/db"
            class="w-full bg-[#3c3c3c] border rounded px-3 py-2 text-sm text-[#cccccc] focus:outline-none transition-colors border-[#3c3c3c] focus:border-[#007acc]"
          />
        </div>

        <!-- Local URI -->
        <div>
          <label class="block text-xs text-[#858585] mb-1.5">Local URI</label>
          <input
            v-model="form.mongo_local_uri"
            type="text"
            placeholder="mongodb://localhost:27017/db_test"
            class="w-full bg-[#3c3c3c] border rounded px-3 py-2 text-sm text-[#cccccc] focus:outline-none transition-colors border-[#3c3c3c] focus:border-[#007acc]"
          />
          <p class="text-[10px] text-[#6e6e6e] mt-1">
            Leave empty to use dump/restore to files instead of live sync.
          </p>
        </div>
      </div>

      <!-- Actions -->
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
          {{ isEditing ? 'Save' : 'Add' }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, watch, computed } from 'vue'

const props = defineProps({
  show: Boolean,
  host: Object,
})

const emit = defineEmits(['save', 'close'])

const isEditing = computed(() => !!props.host)

const form = ref({
  name: '',
  mongo_uri: '',
  mongo_local_uri: '',
})

function resetForm() {
  if (props.host) {
    form.value = {
      name: props.host.name || '',
      mongo_uri: props.host.mongo_uri || '',
      mongo_local_uri: props.host.mongo_local_uri || '',
    }
  } else {
    form.value = {
      name: '',
      mongo_uri: '',
      mongo_local_uri: '',
    }
  }
}

watch(() => props.show, (visible) => {
  if (visible) resetForm()
})

function onClose() {
  emit('close')
}

function onSave() {
  if (!form.value.name.trim() || !form.value.mongo_uri.trim()) {
    return
  }
  emit('save', {
    id: props.host?.id ?? null,
    name: form.value.name.trim(),
    mongo_uri: form.value.mongo_uri.trim(),
    mongo_local_uri: form.value.mongo_local_uri.trim() || null,
  })
}

function onKeydown(e) {
  if (e.key === 'Escape' && props.show) onClose()
}

watch(() => props.show, (visible) => {
  if (visible) {
    window.addEventListener('keydown', onKeydown)
  } else {
    window.removeEventListener('keydown', onKeydown)
  }
})
</script>
