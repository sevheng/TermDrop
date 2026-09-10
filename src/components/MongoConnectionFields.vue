<template>
  <div>
    <label class="block text-xs text-[#858585] mb-1.5">
      Connection String <span class="text-[#f44336]">*</span>
    </label>
    <input
      v-model="uri"
      type="text"
      placeholder="mongodb:// or mongodb+srv://"
      :class="inputClass('uri')"
      @input="$emit('uri-input')"
      @blur="$emit('validate', 'uri')"
      @keydown.enter="$emit('save')"
    />
    <p v-if="errors.uri" class="text-xs text-[#f44336] mt-1">{{ errors.uri }}</p>
  </div>

  <div class="flex gap-3">
    <div class="flex-[2]">
      <label class="block text-xs text-[#858585] mb-1.5">
        Host <span v-if="!isSrv" class="text-[#f44336]">*</span>
      </label>
      <input
        v-model="host"
        type="text"
        :placeholder="hostPlaceholder"
        :class="inputClass('host')"
        @input="$emit('field-input')"
        @blur="$emit('validate', 'host')"
        @keydown.enter="$emit('save')"
      />
      <p v-if="errors.host" class="text-xs text-[#f44336] mt-1">{{ errors.host }}</p>
    </div>
    <div class="flex-1">
      <label class="block text-xs text-[#858585] mb-1.5">
        Port <span v-if="!isSrv" class="text-[#f44336]">*</span>
      </label>
      <input
        v-model.number="port"
        type="number"
        placeholder="27017"
        :class="inputClass('port')"
        @input="$emit('field-input')"
        @blur="$emit('validate', 'port')"
        @keydown.enter="$emit('save')"
      />
      <p v-if="errors.port" class="text-xs text-[#f44336] mt-1">{{ errors.port }}</p>
    </div>
  </div>

  <div class="flex gap-3">
    <div class="flex-1">
      <label class="block text-xs text-[#858585] mb-1.5">Username</label>
      <input
        v-model="username"
        type="text"
        placeholder="user"
        :class="inputClass('username')"
        @input="$emit('field-input')"
        @keydown.enter="$emit('save')"
      />
    </div>
    <div class="flex-1">
      <label class="block text-xs text-[#858585] mb-1.5">Password</label>
      <div class="relative">
        <input
          v-model="password"
          :type="showPassword ? 'text' : 'password'"
          placeholder="password"
          :class="[inputClass('password'), 'pr-8']"
          @input="$emit('field-input')"
          @keydown.enter="$emit('save')"
        />
        <button
          type="button"
          @click="showPassword = !showPassword"
          class="absolute right-2 top-1/2 -translate-y-1/2 text-[#6e6e6e] hover:text-[#cccccc]"
        >
          <component :is="showPassword ? EyeOff : Eye" :size="14" />
        </button>
      </div>
    </div>
  </div>

  <div class="flex gap-3">
    <div class="flex-1">
      <label class="block text-xs text-[#858585] mb-1.5">
        Database
      </label>
      <input
        v-model="database"
        type="text"
        placeholder="database"
        :class="inputClass('database')"
        @input="$emit('field-input')"
        @blur="$emit('validate', 'database')"
        @keydown.enter="$emit('save')"
      />
      <p v-if="errors.database" class="text-xs text-[#f44336] mt-1">{{ errors.database }}</p>
    </div>
    <div class="flex-1">
      <label class="block text-xs text-[#858585] mb-1.5">Auth Source</label>
      <input
        v-model="authSource"
        type="text"
        placeholder="admin"
        :class="inputClass('authSource')"
        @input="$emit('field-input')"
        @keydown.enter="$emit('save')"
      />
    </div>
  </div>

  <div>
    <label class="block text-xs text-[#858585] mb-1.5">Connection Options</label>
    <input
      v-model="options"
      type="text"
      :placeholder="optionsPlaceholder"
      :class="inputClass('options')"
      @input="$emit('field-input')"
      @keydown.enter="$emit('save')"
    />
  </div>
</template>

<script setup>
import { ref, computed } from 'vue'
import { Eye, EyeOff } from 'lucide-vue-next'

/**
 * The connection-string + structured-field group used once for the
 * remote and once for the local MongoDB connection. The parent owns the
 * form values and validation; this component only renders and reports
 * edits: `uri-input` when the URI is typed, `field-input` when any
 * structured field is typed, `validate` on blur, `save` on Enter.
 */
const props = defineProps({
  modelValue: { type: Object, required: true },
  errors: { type: Object, default: () => ({}) },
  isSrv: { type: Boolean, default: false },
  hostPlaceholder: { type: String, default: 'host or IP' },
  optionsPlaceholder: { type: String, default: 'retryWrites=true' },
})

const emit = defineEmits(['update:modelValue', 'uri-input', 'field-input', 'validate', 'save'])

const showPassword = ref(false)

function field(key) {
  return computed({
    get: () => props.modelValue[key],
    set: (value) => emit('update:modelValue', { ...props.modelValue, [key]: value }),
  })
}

const uri = field('uri')
const host = field('host')
const port = field('port')
const username = field('username')
const password = field('password')
const database = field('database')
const authSource = field('authSource')
const options = field('options')

function inputClass(key) {
  const base = 'w-full bg-[#3c3c3c] border rounded px-3 py-2 text-sm text-[#cccccc] focus:outline-none transition-colors'
  const error = props.errors[key] ? 'border-[#f44336] focus:border-[#f44336]' : 'border-[#3c3c3c] focus:border-[#007acc]'
  return `${base} ${error}`
}
</script>
