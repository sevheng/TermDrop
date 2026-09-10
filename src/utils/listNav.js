/**
 * Arrow-key navigation over a list that may contain headers.
 *
 * Generic on purpose — the host sidebar is a tree of collapsible groups, but
 * the same walk serves any list with non-selectable rows in it. Takes plain
 * objects rather than DOM events so it is testable without a DOM, the way
 * selectNav.js already is for SelectMenu.
 */

/** The pixel height of one host row. Shared so the class and any virtual list
 *  measurement cannot disagree. */
export const HOST_ROW_H = 32

/**
 * Flatten groups into the sequence the eye actually sees.
 *
 * Rows inside a collapsed group are absent, not merely hidden — arrowing must
 * skip them, and a filter on the rendered array is the only honest way to say
 * that.
 */
export function visibleRows(groups, collapsed = new Set()) {
  const rows = []
  for (const group of groups ?? []) {
    rows.push({ kind: 'group', name: group.name })
    if (collapsed.has(group.name)) continue
    for (const host of group.hosts ?? []) {
      rows.push({ kind: 'host', host, groupName: group.name })
    }
  }
  return rows
}

/**
 * The next index in `dir`, skipping anything not selectable.
 *
 * Does not wrap: in a long list, arrowing off the end and reappearing at the
 * top loses the reader's place more often than it helps.
 */
export function nextRow(rows, from, dir, { selectable = r => r.kind === 'host' } = {}) {
  const n = rows.length
  if (n === 0) return -1
  let i = from
  for (let step = 0; step < n; step++) {
    i += dir
    if (i < 0 || i >= n) return from >= 0 && from < n && selectable(rows[from]) ? from : -1
    if (selectable(rows[i])) return i
  }
  return from
}

export function firstRow(rows, opts) {
  return nextRow(rows, -1, 1, opts)
}

export function lastRow(rows, opts) {
  return nextRow(rows, rows.length, -1, opts)
}

/** A page of rows, for PageUp/PageDown. */
export function pageRow(rows, from, dir, pageSize = 10, opts) {
  let i = from
  for (let step = 0; step < pageSize; step++) {
    const next = nextRow(rows, i, dir, opts)
    if (next === i) break
    i = next
  }
  return i
}

/**
 * What a key should do, or null when it is not ours.
 *
 * Returned as an intent rather than an index so the caller can act on the
 * tree operations (expand/collapse) it alone knows how to perform.
 */
export function navIntent(key, { onGroup = false, collapsed = false } = {}) {
  switch (key) {
    case 'ArrowDown': return { type: 'move', dir: 1 }
    case 'ArrowUp': return { type: 'move', dir: -1 }
    case 'Home': return { type: 'first' }
    case 'End': return { type: 'last' }
    case 'PageDown': return { type: 'page', dir: 1 }
    case 'PageUp': return { type: 'page', dir: -1 }
    case 'Enter': return { type: 'activate' }
    case 'Delete': return { type: 'delete' }
    case 'ArrowRight':
      // On a collapsed group, open it; anywhere else this is not ours, so a
      // text cursor in the search box still behaves normally.
      return onGroup && collapsed ? { type: 'expand' } : null
    case 'ArrowLeft':
      return onGroup && !collapsed ? { type: 'collapse' } : { type: 'toParent' }
    default:
      return null
  }
}
