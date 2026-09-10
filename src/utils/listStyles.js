/**
 * One treatment for every list and table.
 *
 * Fifteen lists, no two alike: header size, weight and border differed in
 * every instance, separator opacity ran none / 30% / 50% / 60% / full, and the
 * selected state used three different tokens — in RedisKeyTree it was
 * `bg-raised`, the same as hover, so a selected row and a pointed-at row were
 * indistinguishable.
 *
 * Class strings rather than a component, because three of these are real
 * `<table>`s and twelve are div lists; a component cannot span both, but the
 * values can.
 */

/** The header row. Uppercase micro on a tinted ground: a different kind of
 *  thing from the body, rather than a different size of the same thing. */
export const LIST_HEAD = 'text-2xs font-medium text-ink-2 uppercase tracking-wide bg-surface'

/** A header cell. The rule under the header is the strongest line in a list. */
export const LIST_HEAD_CELL = 'text-left font-medium px-2 py-1 border-b border-line whitespace-nowrap'

/** A body row. Hover is `raised`; selection is a different token entirely. */
export const LIST_ROW = 'border-b border-line/50 hover:bg-raised cursor-pointer'

/** A body row that is not interactive — no pointer, no hover. */
export const LIST_ROW_STATIC = 'border-b border-line/50'

/** The selected row. Deliberately not `bg-raised`, or it would collide with
 *  hover; and not `bg-active`, which means "the active tab". */
export const LIST_ROW_SELECTED = 'bg-selected hover:bg-selected'

export const LIST_CELL = 'px-2 py-1 text-xs text-ink'
export const LIST_CELL_MUTED = 'px-2 py-1 text-xs text-ink-2'

/** Any column of numbers. `tabular-nums` stops digits jittering between rows;
 *  there was not one instance of it in the codebase before this. */
export const NUM = 'text-right tabular-nums'

/** The row classes for a given state, joined. */
export function rowClass({ selected = false, interactive = true } = {}) {
  const base = interactive ? LIST_ROW : LIST_ROW_STATIC
  return selected ? `${base} ${LIST_ROW_SELECTED}` : base
}
