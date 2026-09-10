/**
 * Keyboard and placement logic for the custom select.
 *
 * Kept out of the component so it can be tested: @vue/test-utils is not
 * installed, so component logic is otherwise untestable.
 */

/** How long consecutive keystrokes count as one type-ahead query. */
export const TYPEAHEAD_MS = 600

/** Distance kept between the menu and the viewport edge. */
const EDGE_MARGIN = 8

/**
 * The tallest a menu gets, however much room there is.
 *
 * Without this a long host list fills the window and buries the dialog it
 * belongs to. Past roughly this height a list is scrolled rather than read
 * anyway, so the cap costs nothing.
 */
export const MAX_MENU_HEIGHT = 288

/**
 * The next selectable index in `dir`, skipping disabled options and wrapping.
 *
 * Returns -1 when nothing is selectable, so a menu of entirely disabled
 * options cannot spin forever.
 */
export function nextEnabledIndex(options, from, dir) {
  const n = options.length
  if (n === 0) return -1
  for (let step = 1; step <= n; step++) {
    const i = (((from + dir * step) % n) + n) % n
    if (!options[i]?.disabled) return i
  }
  return -1
}

/** The first selectable index, for opening a menu with nothing chosen. */
export function firstEnabledIndex(options) {
  return nextEnabledIndex(options, -1, 1)
}

/** The last selectable index. */
export function lastEnabledIndex(options) {
  return nextEnabledIndex(options, options.length, -1)
}

/** Where a value sits in the list, or -1. */
export function indexOfValue(options, value) {
  return options.findIndex(o => o.value === value)
}

/**
 * Type-ahead: the next option whose label starts with `query`.
 *
 * Searches from just after `from` and wraps, so typing the same letter walks
 * through the options starting with it rather than sticking on the first.
 */
export function typeaheadIndex(options, query, from) {
  const q = String(query || '').toLowerCase()
  if (!q) return -1
  const n = options.length
  for (let step = 1; step <= n; step++) {
    const i = (((from + step) % n) + n) % n
    const o = options[i]
    if (!o?.disabled && String(o.label ?? '').toLowerCase().startsWith(q)) return i
  }
  return -1
}

/**
 * Where to draw the menu relative to its trigger.
 *
 * Fixed coordinates rather than an absolutely-positioned child, so the menu is
 * never clipped by a panel's `overflow: hidden` — which every one of these
 * lives inside. Flips above the trigger when there is more room there, and is
 * capped both by the room available and by MAX_MENU_HEIGHT.
 */
export function menuPlacement(anchor, menuHeight, vh, gap = 4, cap = MAX_MENU_HEIGHT) {
  const wanted = Math.min(menuHeight, cap)
  const below = vh - anchor.bottom - gap - EDGE_MARGIN
  const above = anchor.top - gap - EDGE_MARGIN
  // Prefer below unless it does not fit and above is roomier.
  const placeAbove = wanted > below && above > below
  const maxHeight = Math.max(96, Math.min(cap, Math.floor(placeAbove ? above : below)))
  const height = Math.min(wanted, maxHeight)
  const top = placeAbove ? anchor.top - gap - height : anchor.bottom + gap
  return { top: Math.round(top), maxHeight, placeAbove }
}
