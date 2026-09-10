<template>
  <div class="flex flex-col h-full min-w-0">
    <!-- Header -->
    <div class="flex items-center justify-between px-4 py-3 border-b border-line shrink-0">
      <div class="min-w-0">
        <h3 class="text-sm font-semibold text-ink truncate">
          <span class="text-accent-soft">{{ db }}</span>.{{ collection }}
        </h3>
        <p class="text-2xs text-ink-3 mt-0.5">
          read-only
          <span v-if="stats"> · {{ stats }}</span>
        </p>
      </div>
    </div>

    <!-- Query controls -->
    <div class="px-4 py-3 border-b border-line space-y-2 shrink-0 bg-surface">
      <div class="flex gap-2">
        <div class="flex-1">
          <label class="block text-2xs text-ink-2 mb-1">Filter</label>
          <input
            v-model="filterText"
            spellcheck="false"
            placeholder='{"field": "value"}'
            class="w-full bg-input border rounded px-2 py-1.5 text-xs font-mono text-ink focus:outline-none"
            :class="filterError ? 'border-bad' : 'border-line focus:border-accent'"
            @keydown.ctrl.enter="runQuery(0)"
            @keydown.meta.enter="runQuery(0)"
          />
        </div>
        <div class="w-48">
          <label class="block text-2xs text-ink-2 mb-1">Sort</label>
          <input
            v-model="sortText"
            spellcheck="false"
            placeholder='{"_id": -1}'
            class="w-full bg-input border rounded px-2 py-1.5 text-xs font-mono text-ink focus:outline-none"
            :class="sortError ? 'border-bad' : 'border-line focus:border-accent'"
            @keydown.ctrl.enter="runQuery(0)"
            @keydown.meta.enter="runQuery(0)"
          />
        </div>
        <div class="flex items-end gap-2">
          <div class="flex rounded overflow-hidden border border-line">
            <button
              v-for="mode in ['table', 'json']"
              :key="mode"
              @click="viewMode = mode"
              class="px-2 py-1.5 text-xs capitalize transition-colors"
              :class="viewMode === mode
                ? 'bg-accent-solid text-white'
                : 'bg-input text-ink hover:bg-input-hover'"
            >
              {{ mode }}
            </button>
          </div>
          <button
            @click="runQuery(0)"
            :disabled="loading"
            class="px-3 py-1.5 text-xs rounded bg-accent-solid hover:bg-accent-solid-hover text-white disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {{ loading ? 'Running…' : 'Run' }}
          </button>
        </div>
      </div>

      <!-- A malformed filter is something you iterate on, so it belongs next to
           the box rather than in a toast that disappears. -->
      <p v-if="inputError" class="text-2xs text-bad font-mono">{{ inputError }}</p>
      <p v-else-if="queryError" class="text-2xs text-bad font-mono break-all">{{ queryError }}</p>
      <p v-else class="text-2xs text-ink-3">
        Ctrl/Cmd+Enter to run. A 24-character hex <span class="font-mono">_id</span> is treated as an ObjectId.
      </p>
    </div>

    <!-- Results -->
    <div class="flex-1 overflow-y-auto px-4 py-2">
      <EmptyState v-if="loading" state="loading" title="Querying…" />
      <!-- A filter that matched nothing is not the same as an empty
           collection, and only the caller knows which this is. -->
      <EmptyState
        v-else-if="documents.length === 0"
        :state="filterText.trim() ? 'filtered' : 'empty'"
        :icon="FileSearch"
        :title="rangeLabel"
        :hint="filterText.trim() ? 'No document matches this filter' : undefined"
        :action-label="filterText.trim() ? 'Clear filter' : undefined"
        @action="filterText = ''; runQuery(0)"
      />
      <!-- Table: fields as columns, so documents can be compared at a glance. -->
      <div v-else-if="viewMode === 'table'" class="overflow-x-auto">
        <table class="w-full text-xs font-mono border-collapse">
          <thead class="sticky top-0 bg-surface">
            <tr>
              <th class="w-4 border-b border-line"></th>
              <th
                v-for="col in columns"
                :key="col"
                class="text-left font-medium text-ink-2 px-2 py-1 border-b border-line whitespace-nowrap"
              >
                {{ col }}
              </th>
              <th class="w-6 border-b border-line"></th>
            </tr>
          </thead>
          <tbody>
            <template v-for="(doc, i) in documents" :key="i">
              <tr class="hover:bg-raised cursor-pointer" @click="toggle(i)">
                <td class="px-1 align-top text-ink-2">
                  <ChevronRight
                    :size="10"
                    class="transition-transform"
                    :class="expanded.has(i) ? 'rotate-90' : ''"
                  />
                </td>
                <td
                  v-for="col in columns"
                  :key="col"
                  class="px-2 py-1 text-ink max-w-[16rem] truncate"
                  :title="cellFor(i, col)"
                >
                  {{ cellFor(i, col) }}
                </td>
                <td class="px-1 align-top">
                  <button
                    class="text-ink-2 hover:text-ink"
                    title="Copy document"
                    @click.stop="copyDocument(doc)"
                  >
                    <Copy :size="11" />
                  </button>
                </td>
              </tr>
              <tr v-if="expanded.has(i)">
                <td :colspan="columns.length + 2" class="p-0">
                  <pre
                    class="px-3 py-2 text-xs text-ink bg-canvas overflow-x-auto whitespace-pre"
                  >{{ pretty(doc) }}</pre>
                </td>
              </tr>
            </template>
          </tbody>
        </table>

        <p v-if="hiddenColumns > 0" class="text-2xs text-ink-2 pt-2">
          {{ hiddenColumns }} more field{{ hiddenColumns === 1 ? '' : 's' }} not shown —
          open a row, or switch to JSON, to see everything.
        </p>
        <p v-if="truncated" class="text-2xs text-syn-yellow pt-1">
          Results were cut short because the page exceeded the size limit.
        </p>
      </div>

      <div v-else class="space-y-1">
        <div
          v-for="(doc, i) in documents"
          :key="i"
          class="border border-line/60 rounded overflow-hidden"
        >
          <div
            class="flex items-center gap-2 px-2 py-1 text-xs font-mono cursor-pointer hover:bg-raised"
            @click="toggle(i)"
          >
            <ChevronRight
              :size="11"
              class="transition-transform shrink-0 text-ink-2"
              :class="expanded.has(i) ? 'rotate-90' : ''"
            />
            <span class="flex-1 truncate text-ink">{{ summarize(doc) }}</span>
            <button
              class="text-ink-2 hover:text-ink shrink-0"
              title="Copy document"
              @click.stop="copyDocument(doc)"
            >
              <Copy :size="11" />
            </button>
          </div>
          <pre
            v-if="expanded.has(i)"
            class="px-3 py-2 text-xs font-mono text-ink bg-canvas overflow-x-auto whitespace-pre"
          >{{ pretty(doc) }}</pre>
        </div>

        <p v-if="truncated" class="text-2xs text-syn-yellow pt-1">
          Results were cut short because the page exceeded the size limit.
        </p>
      </div>
    </div>

    <!-- Pagination -->
    <div class="flex items-center justify-between px-4 py-2.5 border-t border-line shrink-0 bg-surface">
      <span class="text-2xs text-ink-3">
        {{ rangeLabel }}
        <span v-if="elapsedMs !== null"> · {{ elapsedMs }} ms</span>
      </span>
      <div class="flex items-center gap-2">
        <label class="text-2xs text-ink-2">
          Page size
          <SelectMenu
            size="xs"
            class="ml-1"
            v-model="pageSize"
            :options="pageSizeOptions"
            @change="runQuery(0)"
          />
        </label>
        <button
          @click="runQuery(page - 1)"
          :disabled="loading || page === 0"
          class="px-2 py-1 text-xs rounded text-ink hover:bg-raised disabled:opacity-40 disabled:cursor-not-allowed"
        >
          Prev
        </button>
        <button
          @click="runQuery(page + 1)"
          :disabled="loading || page >= maxPage"
          class="px-2 py-1 text-xs rounded text-ink hover:bg-raised disabled:opacity-40 disabled:cursor-not-allowed"
        >
          Next
        </button>
      </div>
    </div>
  </div>
</template>


<script setup>
import { ref, computed, watch } from 'vue'
import { ChevronRight, Copy, FileSearch } from 'lucide-vue-next'
import EmptyState from './EmptyState.vue'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { invoke } from '../utils/invoke.js'
import { toast } from '../utils/toast.js'
import { prettyPrintDocument, summarizeDocument } from '../utils/bsonDisplay.js'
import { parseDocuments, deriveColumns, cellText } from '../utils/mongoTable.js'
import SelectMenu from './SelectMenu.vue'
import {
  validateJsonInput,
  clampPageSize,
  skipFor,
  lastPage,
  describeRange,
  DEFAULT_PAGE_SIZE,
} from '../utils/mongoQuery.js'

const props = defineProps({
  hostId: { type: Number, required: true },
  db: { type: String, default: '' },
  collection: { type: String, default: '' },
})

const viewMode = ref('table')
const filterText = ref('')
const sortText = ref('')
const pageSize = ref(DEFAULT_PAGE_SIZE)
const pageSizeOptions = [25, 50, 100, 200].map(n => ({ value: n, label: String(n) }))
const page = ref(0)

const documents = ref([])
const expanded = ref(new Set())
const loading = ref(false)
const truncated = ref(false)
const elapsedMs = ref(null)
const queryError = ref('')
const total = ref(0)
const estimated = ref(false)
const stats = ref('')

const filterError = computed(() => validateJsonInput(filterText.value, 'filter'))
const sortError = computed(() => validateJsonInput(sortText.value, 'sort'))
const inputError = computed(() => filterError.value || sortError.value)
const maxPage = computed(() => lastPage(total.value, pageSize.value))
const rangeLabel = computed(() =>
  describeRange(page.value, pageSize.value, documents.value.length, total.value, estimated.value),
)

/**
 * The page parsed once for the table, rather than per cell. Documents in a
 * collection need not share a shape, so the columns are the union of what this
 * page actually contains.
 */
const parsedDocs = computed(() => parseDocuments(documents.value))
const derived = computed(() => deriveColumns(parsedDocs.value))
const columns = computed(() => derived.value.columns)
const hiddenColumns = computed(() => derived.value.hidden)

/** One cell, blank when this document simply lacks the field. */
function cellFor(index, column) {
  return cellText(parsedDocs.value[index]?.[column])
}

function summarize(doc) {
  return summarizeDocument(doc)
}

function pretty(doc) {
  return prettyPrintDocument(doc)
}

function toggle(i) {
  const next = new Set(expanded.value)
  if (next.has(i)) next.delete(i)
  else next.add(i)
  expanded.value = next
}

async function copyDocument(doc) {
  try {
    await writeText(prettyPrintDocument(doc))
    toast('Document copied', 'success')
  } catch (err) {
    toast(`Could not copy: ${err}`, 'error')
  }
}

async function runQuery(targetPage) {
  if (inputError.value || loading.value) return

  const wanted = Math.max(0, targetPage)
  loading.value = true
  queryError.value = ''
  try {
    const args = {
      hostId: props.hostId,
      db: props.db,
      collection: props.collection,
      filter: filterText.value.trim() || null,
      sort: sortText.value.trim() || null,
      projection: null,
    }

    // Count first so the page can be clamped to something that exists.
    const counted = await invoke('mongodb_count', {
      hostId: args.hostId,
      db: args.db,
      collection: args.collection,
      filter: args.filter,
    })
    total.value = counted.count
    estimated.value = counted.estimated

    const clamped = Math.min(wanted, lastPage(counted.count, pageSize.value))
    const result = await invoke('mongodb_find', {
      ...args,
      skip: skipFor(clamped, pageSize.value),
      limit: clampPageSize(pageSize.value),
    })

    documents.value = result.documents
    truncated.value = result.truncated
    elapsedMs.value = result.elapsed_ms
    page.value = clamped
    expanded.value = new Set()
  } catch (err) {
    // Inline, not a toast: a bad query is something you correct in place.
    queryError.value = String(err)
    documents.value = []
    truncated.value = false
  } finally {
    loading.value = false
  }
}

async function loadStats() {
  stats.value = ''
  try {
    const raw = await invoke('mongodb_collection_stats', {
      hostId: props.hostId,
      db: props.db,
      collection: props.collection,
    })
    const s = JSON.parse(raw)
    const num = (v) => (v && typeof v === 'object' ? Number(Object.values(v)[0]) : Number(v))
    const size = num(s.size)
    const indexes = num(s.nindexes)
    if (Number.isFinite(size)) {
      stats.value = `${formatBytes(size)}, ${indexes} index${indexes === 1 ? '' : 'es'}`
    }
  } catch {
    // Stats are a nicety; a collection is still browsable without them.
  }
}

function formatBytes(bytes) {
  if (!Number.isFinite(bytes)) return ''
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit++
  }
  return `${value < 10 && unit > 0 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`
}

watch(
  () => [props.db, props.collection],
  () => {
    if (!props.collection) return
    filterText.value = ''
    sortText.value = ''
    page.value = 0
    documents.value = []
    queryError.value = ''
    elapsedMs.value = null
    runQuery(0)
    loadStats()
  },
  { immediate: true },
)
</script>
