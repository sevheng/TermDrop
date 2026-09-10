/**
 * Paging and pattern helpers for the key browser.
 *
 * Kept out of the components so they can be tested: `@vue/test-utils` is not
 * installed, so component logic is otherwise untestable.
 */

/**
 * Escape a literal string for use inside a Redis glob pattern.
 *
 * Not optional. Redis keys may contain `*`, `?`, `[`, `]` and `\`, and clicking
 * a prefix group builds a pattern from a real key's bytes — unescaped, a
 * prefix like `report[2024]` would match nothing while a prefix like `a*` would
 * match half the keyspace.
 */
export function escapeGlob(literal) {
  return String(literal ?? '').replace(/[\\*?[\]]/g, ch => `\\${ch}`)
}

/** The MATCH pattern for one prefix group. */
export function prefixPattern(prefix, delimiter = ':') {
  return `${escapeGlob(prefix)}${delimiter}*`
}

/**
 * A forward-only cursor stack.
 *
 * Redis cursors cannot be rewound and carry no page number, so going back means
 * remembering where each page started. Cursors are strings all the way through:
 * they are u64 server-side and would lose precision as a JS number.
 */
export function createCursorStack() {
  let stack = ['0']

  return {
    /** The cursor the current page starts at. */
    current: () => stack[stack.length - 1],
    /** Record the cursor the next page starts at. */
    push(cursor) {
      stack = [...stack, String(cursor)]
    },
    /** Step back one page, returning the cursor to load. */
    back() {
      if (stack.length > 1) stack = stack.slice(0, -1)
      return stack[stack.length - 1]
    },
    reset() {
      stack = ['0']
    },
    canGoBack: () => stack.length > 1,
    /** 1-based, for display. */
    pageNumber: () => stack.length,
  }
}

/**
 * How a TTL should read.
 *
 * `-1` and `-2` are Redis's own sentinels, and rendering either as a duration
 * would be actively misleading.
 */
export function formatTtl(ms) {
  if (ms === -1 || ms == null) return 'no expiry'
  if (ms === -2) return 'expired'
  if (ms < 1000) return `${ms}ms`

  const total = Math.floor(ms / 1000)
  const days = Math.floor(total / 86400)
  const hours = Math.floor((total % 86400) / 3600)
  const minutes = Math.floor((total % 3600) / 60)
  const seconds = total % 60

  const parts = []
  if (days) parts.push(`${days}d`)
  if (hours) parts.push(`${hours}h`)
  if (minutes) parts.push(`${minutes}m`)
  // Seconds are dropped once the value is big enough that they are noise.
  if (seconds && !days && !hours) parts.push(`${seconds}s`)
  return parts.join(' ') || '0s'
}

/** A key type's display name. */
export function formatKeyKind(kind) {
  return (
    {
      string: 'String',
      list: 'List',
      set: 'Set',
      zset: 'Sorted set',
      hash: 'Hash',
      stream: 'Stream',
      none: 'Gone',
    }[kind] || kind || 'Unknown'
  )
}

/** Whether TermDrop has a viewer for this type. */
export function isViewableKind(kind) {
  return ['string', 'list', 'set', 'zset', 'hash', 'stream'].includes(kind)
}

/**
 * How the scan's progress should read.
 *
 * Deliberately says "of ~N" and never claims completeness while the cursor is
 * still open: SCAN gives no total, and DBSIZE is only an estimate of what a
 * filtered scan will find.
 */
export function summarizeScan(loaded, totalKeys, done) {
  if (done) return `${loaded.toLocaleString()} ${loaded === 1 ? 'key' : 'keys'}`
  if (!totalKeys) return `${loaded.toLocaleString()} so far`
  return `${loaded.toLocaleString()} of ~${totalKeys.toLocaleString()}`
}
