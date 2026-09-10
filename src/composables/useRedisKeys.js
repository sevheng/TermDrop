import { ref, computed } from 'vue'
import { invoke } from '../utils/invoke.js'
import { createCursorStack, summarizeScan } from '../utils/redisKeys.js'
import { MAX_KEYS_PER_PAGE } from '../utils/redisLimits.js'

/**
 * One database's keyspace, paged by SCAN.
 *
 * Paging is forward-only by cursor, because that is what SCAN offers: there is
 * no page number, no total, and no way to jump. Going back replays a
 * remembered cursor rather than computing an offset.
 *
 * The cursor a page returns is where the *next* page starts, so it is held as
 * `pending` until the user actually asks for that page.
 */
export function useRedisKeys(hostId, db, serverInfo) {
  const keys = ref([])
  const loading = ref(false)
  const error = ref('')
  const pattern = ref('')
  const typeFilter = ref('')
  const withMemory = ref(false)
  const done = ref(false)
  const pageNumber = ref(1)
  const canGoBack = ref(false)

  let stack = createCursorStack()
  let pending = null

  const dbKeyCount = computed(
    () => serverInfo.value?.databases?.find(d => d.index === db.value)?.keys ?? 0,
  )

  const summary = computed(() =>
    summarizeScan(keys.value.length, dbKeyCount.value, done.value && pageNumber.value === 1),
  )

  async function load(cursor) {
    if (!hostId.value) return
    loading.value = true
    // Errors render inline, not as a toast: a bad pattern is something you
    // correct in place, and a toast would scroll away from the box you fix.
    error.value = ''
    try {
      const page = await invoke('redis_scan_keys', {
        hostId: hostId.value,
        db: db.value,
        cursor,
        pattern: pattern.value || null,
        typeFilter: typeFilter.value || null,
        limit: MAX_KEYS_PER_PAGE,
        withMemory: withMemory.value,
        supportsScanType: serverInfo.value?.supports_scan_type ?? false,
      })
      keys.value = page.keys
      done.value = page.done
      pending = page.done ? null : page.cursor
    } catch (err) {
      error.value = String(err)
      keys.value = []
      pending = null
    } finally {
      loading.value = false
      pageNumber.value = stack.pageNumber()
      canGoBack.value = stack.canGoBack()
    }
  }

  /** Start the iteration over, after a pattern, filter or database change. */
  async function restart() {
    stack = createCursorStack()
    pending = null
    done.value = false
    await load('0')
  }

  const hasNext = computed(() => !done.value && !loading.value)

  async function forward() {
    if (pending == null) return
    const cursor = pending
    stack.push(cursor)
    await load(cursor)
  }

  async function back() {
    if (!stack.canGoBack()) return
    await load(stack.back())
  }

  function reset() {
    stack = createCursorStack()
    pending = null
    keys.value = []
    pattern.value = ''
    typeFilter.value = ''
    error.value = ''
    done.value = false
    pageNumber.value = 1
    canGoBack.value = false
  }

  return {
    keys,
    loading,
    error,
    pattern,
    typeFilter,
    withMemory,
    done,
    hasNext,
    canGoBack,
    pageNumber,
    summary,
    restart,
    back,
    forward,
    reset,
  }
}
