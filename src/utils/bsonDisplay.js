/**
 * Rendering canonical extended JSON for people to read.
 *
 * The backend returns canonical extended JSON so no numeric type is lost in
 * transit. Canonical is precise but unreadable — an ObjectId arrives as
 * `{"$oid": "..."}` and a date as `{"$date": {"$numberLong": "..."}}` — so it
 * is unwrapped here, at the last moment, for display only.
 */

/** Depth past which nested values are elided rather than expanded. */
const DEFAULT_MAX_DEPTH = 12

/**
 * Unwrap one extended-JSON wrapper, returning a display string, or null when
 * the value is not a wrapper.
 */
export function unwrapExtendedJson(value) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return null

  const keys = Object.keys(value)
  if (keys.length === 0) return null

  switch (keys[0]) {
    case '$oid':
      // Single quotes: these strings get JSON.stringify'd for the pretty view,
      // and double quotes would come back escaped as ObjectId(\"...\").
      return `ObjectId('${value.$oid}')`
    case '$date': {
      const raw = value.$date
      const ms = typeof raw === 'object' && raw !== null ? Number(raw.$numberLong) : raw
      const date = new Date(ms)
      return Number.isNaN(date.getTime())
        ? `Date(${String(ms)})`
        : `ISODate('${date.toISOString()}')`
    }
    case '$numberLong':
      return value.$numberLong
    case '$numberInt':
      return value.$numberInt
    case '$numberDouble':
      return value.$numberDouble
    case '$numberDecimal':
      // Kept as text: the whole point of Decimal128 is that a JS number
      // cannot represent it.
      return value.$numberDecimal
    case '$binary':
      return `Binary('${value.$binary?.subType ?? ''}')`
    case '$timestamp':
      return `Timestamp(${value.$timestamp?.t ?? 0}, ${value.$timestamp?.i ?? 0})`
    case '$regularExpression':
      return `/${value.$regularExpression?.pattern ?? ''}/${value.$regularExpression?.options ?? ''}`
    case '$minKey':
      return 'MinKey'
    case '$maxKey':
      return 'MaxKey'
    case '$undefined':
      return 'undefined'
    default:
      return null
  }
}

/** Recursively replace extended-JSON wrappers with display strings. */
export function toDisplayValue(value, depth = 0, maxDepth = DEFAULT_MAX_DEPTH) {
  if (depth > maxDepth) return '…'

  const unwrapped = unwrapExtendedJson(value)
  if (unwrapped !== null) return unwrapped

  if (Array.isArray(value)) {
    return value.map(v => toDisplayValue(v, depth + 1, maxDepth))
  }
  if (value !== null && typeof value === 'object') {
    const out = {}
    for (const [k, v] of Object.entries(value)) {
      out[k] = toDisplayValue(v, depth + 1, maxDepth)
    }
    return out
  }
  return value
}

/** A document as indented, readable JSON. Returns the input if unparseable. */
export function prettyPrintDocument(json, maxDepth = DEFAULT_MAX_DEPTH) {
  try {
    return JSON.stringify(toDisplayValue(JSON.parse(json), 0, maxDepth), null, 2)
  } catch {
    return json
  }
}

/**
 * A single-line summary for a collapsed row, truncated to `limit`.
 * Always leads with `_id` when there is one, since that is what identifies it.
 */
export function summarizeDocument(json, limit = 120) {
  let parsed
  try {
    parsed = toDisplayValue(JSON.parse(json))
  } catch {
    return json.slice(0, limit)
  }
  if (parsed === null || typeof parsed !== 'object') return String(parsed).slice(0, limit)

  const entries = Object.entries(parsed)
  entries.sort(([a], [b]) => (a === '_id' ? -1 : b === '_id' ? 1 : 0))

  const parts = []
  for (const [k, v] of entries) {
    const rendered = typeof v === 'object' && v !== null
      ? Array.isArray(v) ? `[${v.length}]` : '{…}'
      : String(v)
    parts.push(`${k}: ${rendered}`)
    if (parts.join(', ').length > limit) break
  }

  const line = parts.join(', ')
  return line.length > limit ? line.slice(0, limit - 1) + '…' : line
}
