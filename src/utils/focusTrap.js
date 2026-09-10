/**
 * Keeping Tab inside an open dialog.
 *
 * ModalShell had no trap and no initial focus, so Tab walked out of a dialog
 * and into the application behind it — where the controls are covered by a
 * scrim and cannot be seen, let alone understood. Eleven modals use it.
 */

/** What the browser will actually focus, in document order. */
export const FOCUSABLE = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled]):not([type="hidden"])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(',')

/** Whether an element occupies any space on screen. */
function hasBoxes(el) {
  return el.offsetWidth > 0 || el.offsetHeight > 0 || (el.getClientRects?.().length ?? 0) > 0
}

/**
 * Drop anything present but unreachable — `display:none` yields no boxes.
 *
 * Fails *open*. Where no element reports a box, layout is unavailable rather
 * than everything being hidden, and filtering on that would empty the list and
 * disable the trap completely. Including a hidden control costs one stray Tab
 * stop; excluding every control lets Tab escape the dialog, which is the bug
 * this exists to prevent.
 */
export function visibleOnly(elements) {
  const all = Array.from(elements ?? [])
  const visible = all.filter(hasBoxes)
  return visible.length > 0 || all.length === 0 ? visible : all
}

/**
 * Where Tab should land next, wrapping at the ends.
 *
 * Unlike list navigation this *does* wrap, because a dialog is a closed loop:
 * falling off the end has nowhere to go but back to the application.
 */
export function nextFocusIndex(count, from, dir) {
  if (count <= 0) return -1
  if (from < 0) return dir > 0 ? 0 : count - 1
  return (((from + dir) % count) + count) % count
}
