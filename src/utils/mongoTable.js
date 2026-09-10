/**
 * Turning a page of documents into table columns and cells.
 *
 * A MongoDB collection has no schema, so the columns are whatever the documents
 * on the current page happen to contain. Deriving them here, rather than in the
 * component, is what makes the behaviour testable — MongoDocumentsView.vue
 * cannot be mounted in tests because @vue/test-utils is not installed.
 */

import { toDisplayValue } from './bsonDisplay.js'

/** Beyond this many columns a table stops being readable and starts scrolling forever. */
export const DEFAULT_MAX_COLUMNS = 12

/**
 * Parse a page of canonical extended JSON into display-ready objects.
 *
 * A document that will not parse becomes null rather than throwing, so one bad
 * row cannot blank the whole page; callers render those from the raw string.
 */
export function parseDocuments(jsonStrings) {
  return (jsonStrings || []).map((json) => {
    try {
      const parsed = toDisplayValue(JSON.parse(json))
      // Only an object has fields to spread across columns.
      return parsed !== null && typeof parsed === 'object' && !Array.isArray(parsed)
        ? parsed
        : null
    } catch {
      return null
    }
  })
}

/**
 * The columns for a page: `_id` first, then the fields most documents share.
 *
 * Ordering by how many documents contain a field keeps the useful columns on
 * the left when documents have different shapes; first-seen order breaks ties
 * so the result is stable rather than dependent on object key iteration.
 */
export function deriveColumns(docs, maxColumns = DEFAULT_MAX_COLUMNS) {
  const counts = new Map()
  const firstSeen = new Map()

  let position = 0
  for (const doc of docs || []) {
    if (!doc) continue
    for (const field of Object.keys(doc)) {
      counts.set(field, (counts.get(field) || 0) + 1)
      if (!firstSeen.has(field)) firstSeen.set(field, position++)
    }
  }

  const fields = [...counts.keys()].filter((f) => f !== '_id')
  fields.sort((a, b) => {
    const byCount = counts.get(b) - counts.get(a)
    return byCount !== 0 ? byCount : firstSeen.get(a) - firstSeen.get(b)
  })

  // `_id` identifies the row, so it leads regardless of how often it appears.
  const ordered = counts.has('_id') ? ['_id', ...fields] : fields
  const limit = Math.max(1, maxColumns)
  return { columns: ordered.slice(0, limit), hidden: Math.max(0, ordered.length - limit) }
}

/**
 * One cell's text. Nested values are collapsed rather than expanded — the row
 * can be opened to see the whole document.
 */
export function cellText(value) {
  if (value === undefined) return ''
  if (value === null) return 'null'
  if (Array.isArray(value)) return `[${value.length}]`
  if (typeof value === 'object') return '{…}'
  return String(value)
}
