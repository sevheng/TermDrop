/**
 * Pure helpers for the document browser's query controls.
 *
 * Kept out of the component so they can be tested; @vue/test-utils is not
 * installed, so component logic is otherwise untestable.
 */

/** Must match MAX_FIND_LIMIT in src-tauri/src/mongodb.rs. */
export const MAX_PAGE_SIZE = 200
export const DEFAULT_PAGE_SIZE = 25

/**
 * Validate a filter/sort/projection box before it reaches the backend, so a
 * typo is reported next to the field instead of as a round trip.
 * Returns null when valid, or a message.
 */
export function validateJsonInput(text, field = 'filter') {
  const trimmed = (text || '').trim()
  if (!trimmed) return null
  let parsed
  try {
    parsed = JSON.parse(trimmed)
  } catch (err) {
    return `${field} is not valid JSON: ${err.message}`
  }
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
    return `${field} must be a JSON object, e.g. {"field": "value"}`
  }
  return null
}

/** Clamp a requested page size to what the backend will actually return. */
export function clampPageSize(size) {
  const n = Number(size)
  if (!Number.isFinite(n) || n < 1) return DEFAULT_PAGE_SIZE
  return Math.min(Math.floor(n), MAX_PAGE_SIZE)
}

/** How many documents to skip for a zero-based page. */
export function skipFor(page, pageSize) {
  const safePage = Math.max(0, Math.floor(Number(page) || 0))
  return safePage * clampPageSize(pageSize)
}

/**
 * The last page index for a total, zero-based. A total of 0 still has page 0,
 * so Next/Prev have something coherent to clamp against.
 */
export function lastPage(total, pageSize) {
  const size = clampPageSize(pageSize)
  if (!Number.isFinite(Number(total)) || total <= 0) return 0
  return Math.max(0, Math.ceil(total / size) - 1)
}

/** "1–25 of ~10,000,000", or an empty-state message. */
export function describeRange(page, pageSize, returned, total, estimated) {
  if (returned === 0) return total === 0 ? 'No documents' : 'No documents on this page'
  const start = skipFor(page, pageSize) + 1
  const end = start + returned - 1
  const totalText = `${estimated ? '~' : ''}${Number(total).toLocaleString()}`
  return `${start.toLocaleString()}–${end.toLocaleString()} of ${totalText}`
}
