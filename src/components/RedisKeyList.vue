<template>
  <div class="flex flex-col h-full min-w-0">
    <!-- Filter bar -->
    <div class="flex items-center gap-2 px-3 py-2 border-b border-[#3c3c3c] shrink-0">
      <div class="relative flex-1 min-w-0">
        <Search :size="12" class="absolute left-2 top-1/2 -translate-y-1/2 text-[#6e6e6e]" />
        <input
          :value="pattern"
          @input="$emit('update:pattern', $event.target.value)"
          @keyup.enter="$emit('search')"
          placeholder="Match pattern, e.g. user:*"
          class="w-full bg-[#3c3c3c] text-[#cccccc] text-xs rounded pl-7 pr-2 py-1 outline-none focus:ring-1 focus:ring-[#007acc]"
        />
      </div>
      <select
        :value="typeFilter"
        @change="$emit('update:typeFilter', $event.target.value); $emit('search')"
        class="bg-[#3c3c3c] text-[#cccccc] text-xs rounded px-2 py-1 outline-none"
      >
        <option value="">All types</option>
        <option v-for="t in TYPES" :key="t" :value="t">{{ formatKeyKind(t) }}</option>
      </select>
      <button
        @click="$emit('search')"
        class="text-xs px-2 py-1 rounded bg-[#0e639c] hover:bg-[#1177bb] text-white"
      >
        Search
      </button>
    </div>

    <!--
      Inline, not a toast: a bad pattern is something you correct in place, and
      a toast would scroll away from the box you fix it in.
    -->
    <div v-if="error" class="px-3 py-2 text-xs text-red-400 border-b border-[#3c3c3c] shrink-0">
      {{ error }}
    </div>

    <div class="flex-1 overflow-auto">
      <div v-if="loading" class="flex items-center justify-center py-8">
        <Loader2 :size="16" class="animate-spin text-[#858585]" />
      </div>
      <div
        v-else-if="keys.length === 0"
        class="flex flex-col items-center justify-center py-10 text-[#6e6e6e]"
      >
        <Search :size="20" class="mb-2 opacity-50" />
        <p class="text-xs">No keys on this page</p>
        <p v-if="!done" class="text-[10px] mt-1">
          SCAN returns pages, not results — there may be more further on.
        </p>
      </div>
      <table v-else class="w-full text-xs">
        <thead class="sticky top-0 bg-[#252526] text-[#858585]">
          <tr>
            <th class="text-left font-normal px-3 py-1.5">Key</th>
            <th class="text-left font-normal px-2 py-1.5 w-24">Type</th>
            <th class="text-left font-normal px-2 py-1.5 w-28">TTL</th>
            <th v-if="showMemory" class="text-right font-normal px-3 py-1.5 w-24">Size</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="k in keys"
            :key="k.key_b64"
            @click="$emit('open-key', k)"
            class="cursor-pointer hover:bg-[#2a2d2e] border-t border-[#3c3c3c]/30"
            :class="k.key_b64 === activeKeyB64 ? 'bg-[#37373d]' : ''"
          >
            <td class="px-3 py-1 font-mono truncate max-w-0 text-[#cccccc]">
              <span v-if="k.key.binary" class="text-[#d19a66]" title="not valid UTF-8">
                ⟨binary {{ k.key.bytes }} bytes⟩
              </span>
              <span v-else>{{ k.key.text }}</span>
            </td>
            <td class="px-2 py-1 text-[#858585]">{{ formatKeyKind(k.kind) }}</td>
            <td class="px-2 py-1 text-[#858585]">{{ formatTtl(k.ttl_ms) }}</td>
            <td v-if="showMemory" class="px-3 py-1 text-right text-[#858585]">
              {{ k.memory_bytes == null ? '—' : formatBytes(k.memory_bytes) }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- Paging -->
    <div
      class="flex items-center justify-between px-3 py-1.5 border-t border-[#3c3c3c] text-[11px] text-[#858585] shrink-0"
    >
      <span>{{ summary }}</span>
      <div class="flex items-center gap-2">
        <label class="flex items-center gap-1 cursor-pointer" title="Costs an extra command per key">
          <input
            type="checkbox"
            :checked="showMemory"
            @change="$emit('update:showMemory', $event.target.checked); $emit('search')"
            class="accent-[#007acc]"
          />
          Sizes
        </label>
        <button
          :disabled="!canGoBack || loading"
          @click="$emit('back')"
          class="px-2 py-0.5 rounded hover:bg-[#2a2d2e] disabled:opacity-40 disabled:hover:bg-transparent"
        >
          ‹ prev
        </button>
        <span>page {{ pageNumber }}</span>
        <button
          :disabled="!hasNext || loading"
          @click="$emit('forward')"
          class="px-2 py-0.5 rounded hover:bg-[#2a2d2e] disabled:opacity-40 disabled:hover:bg-transparent"
        >
          next ›
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
/**
 * A SCAN-paged list of keys.
 *
 * There is no page count and no "last page" button, because SCAN cannot
 * provide either: it hands out a cursor, and the only honest controls are
 * "the page before this one" and "the next one".
 */
import { Search, Loader2 } from 'lucide-vue-next'
import { formatKeyKind, formatTtl } from '../utils/redisKeys.js'
import { formatBytes } from '../utils/format.js'

const TYPES = ['string', 'list', 'set', 'zset', 'hash', 'stream']

defineProps({
  keys: { type: Array, required: true },
  pattern: { type: String, default: '' },
  typeFilter: { type: String, default: '' },
  showMemory: { type: Boolean, default: false },
  loading: { type: Boolean, default: false },
  error: { type: String, default: '' },
  done: { type: Boolean, default: false },
  hasNext: { type: Boolean, default: false },
  canGoBack: { type: Boolean, default: false },
  pageNumber: { type: Number, default: 1 },
  summary: { type: String, default: '' },
  activeKeyB64: { type: String, default: null },
})

defineEmits([
  'update:pattern',
  'update:typeFilter',
  'update:showMemory',
  'search',
  'open-key',
  'back',
  'forward',
])
</script>
