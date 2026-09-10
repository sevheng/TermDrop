<template>
  <div class="flex flex-col h-full border-l border-line min-w-0">
    <div v-if="!keyB64" class="flex flex-col items-center justify-center h-full text-ink-3">
      <FileText :size="20" class="mb-2 opacity-50" />
      <p class="text-xs">Select a key to view it</p>
    </div>

    <template v-else>
      <div class="px-3 py-2 border-b border-line shrink-0">
        <p class="font-mono text-xs text-ink break-all">{{ keyLabel }}</p>
        <div class="flex items-center gap-3 mt-1 text-[10px] text-ink-2">
          <span>{{ formatKeyKind(page?.kind) }}</span>
          <span>{{ formatTtl(page?.ttl_ms) }}</span>
          <span v-if="page?.encoding">{{ page.encoding }}</span>
          <span v-if="page?.total != null">
            {{ page.total.toLocaleString() }} {{ page.kind === 'string' ? 'bytes' : 'elements' }}
          </span>
        </div>
      </div>

      <div v-if="loading" class="flex items-center justify-center py-8">
        <Loader2 :size="16" class="animate-spin text-ink-2" />
      </div>
      <div v-else-if="error" class="px-3 py-2 text-xs text-red-400">{{ error }}</div>

      <div
        v-else-if="!isViewableKind(page?.kind)"
        class="flex flex-col items-center justify-center flex-1 text-ink-3 px-4 text-center"
      >
        <AlertCircle :size="20" class="mb-2 opacity-50" />
        <p class="text-xs">
          {{ page?.kind === 'none' ? 'This key no longer exists.' : 'TermDrop cannot display this type.' }}
        </p>
        <p v-if="page?.kind && page.kind !== 'none'" class="text-[10px] mt-1">
          {{ page.kind }} — usually a Redis module type.
        </p>
      </div>

      <div v-else class="flex-1 overflow-auto">
        <!--
          Above the value and sticky, never after it. A 256 KB preview is
          thousands of lines tall, so a note underneath is unreachable — the
          user sees what looks like a complete value and scrolls away none the
          wiser, which is exactly what this note exists to prevent.
        -->
        <p
          v-if="truncatedNote"
          class="sticky top-0 z-10 px-3 py-1.5 text-[10px] text-warn-soft bg-warn-bg border-b border-warn-line"
        >
          {{ truncatedNote }}
        </p>

        <!-- A string is one value; everything else is a table of elements. -->
        <pre
          v-if="page.kind === 'string'"
          class="px-3 py-2 text-xs font-mono text-ink whitespace-pre-wrap break-all"
          >{{ display(page.entries[0]?.value) }}</pre
        >
        <table v-else class="w-full text-xs">
          <thead class="sticky top-0 bg-surface text-ink-2">
            <tr>
              <th v-if="hasField" class="text-left font-normal px-3 py-1.5 w-1/3">
                {{ page.kind === 'stream' ? 'ID' : page.kind === 'list' ? '#' : 'Field' }}
              </th>
              <th class="text-left font-normal px-3 py-1.5">Value</th>
              <th v-if="page.kind === 'zset'" class="text-right font-normal px-3 py-1.5 w-24">
                Score
              </th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="(entry, i) in page.entries"
              :key="i"
              class="border-t border-line/30 align-top"
            >
              <td v-if="hasField" class="px-3 py-1 font-mono text-syn-cyan break-all">
                {{ display(entry.field) }}
              </td>
              <td class="px-3 py-1 font-mono text-ink break-all">
                {{ display(entry.value) }}
              </td>
              <td v-if="page.kind === 'zset'" class="px-3 py-1 text-right text-ink-2">
                {{ entry.score }}
              </td>
            </tr>
          </tbody>
        </table>

      </div>

      <div
        v-if="isViewableKind(page?.kind) && page.kind !== 'string'"
        class="flex items-center justify-between px-3 py-1.5 border-t border-line text-[11px] text-ink-2 shrink-0"
      >
        <span>{{ page.entries.length }} shown</span>
        <button
          v-if="!page.done || canPageByOffset"
          :disabled="loading"
          @click="$emit('more')"
          class="px-2 py-0.5 rounded hover:bg-raised disabled:opacity-40"
        >
          Load more
        </button>
      </div>
    </template>
  </div>
</template>

<script setup>
/**
 * A read-only, type-aware view of one key.
 *
 * Every type is paged or windowed by the backend, so opening a ten-million
 * element set costs the same as opening an empty one.
 */
import { computed } from 'vue'
import { FileText, Loader2, AlertCircle } from 'lucide-vue-next'
import { formatKeyKind, formatTtl, isViewableKind } from '../utils/redisKeys.js'
import { formatBytes } from '../utils/format.js'

const props = defineProps({
  keyB64: { type: String, default: null },
  keyLabel: { type: String, default: '' },
  page: { type: Object, default: null },
  loading: { type: Boolean, default: false },
  error: { type: String, default: '' },
})

defineEmits(['more'])

const hasField = computed(() => ['hash', 'list', 'stream'].includes(props.page?.kind))

/** List and sorted set page by index, so there is always a next window. */
const canPageByOffset = computed(() => ['list', 'zset'].includes(props.page?.kind))

/**
 * The text to render. The backend already puts base64 in `text` when the bytes
 * are not valid UTF-8, so there is nothing to choose here — `binary` only
 * drives the note at the bottom.
 */
function display(bytes) {
  return bytes?.text ?? ''
}

const truncatedNote = computed(() => {
  const value = props.page?.entries?.[0]?.value
  if (props.page?.kind === 'string' && value?.truncated) {
    // Byte length, not character count: a multi-byte value would otherwise
    // report a preview smaller than it actually is.
    const shown = new TextEncoder().encode(value.text).length
    return `Truncated — showing the first ${formatBytes(shown)} of ${formatBytes(value.bytes)}.`
  }
  const anyBinary = props.page?.entries?.some(e => e.value?.binary || e.field?.binary)
  return anyBinary ? 'Values that are not valid UTF-8 are shown as base64.' : ''
})
</script>
