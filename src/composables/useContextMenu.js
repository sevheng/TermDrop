import { ref, nextTick } from 'vue'

const EDGE_MARGIN = 8

/**
 * Keep a menu of size `rect` at (x, y) inside a viewport of vw x vh.
 * The menu is only moved when it would overflow, then kept EDGE_MARGIN
 * from the edge.
 */
export function clampToViewport(x, y, rect, vw, vh) {
  if (x + rect.width > vw) x = vw - rect.width - EDGE_MARGIN
  if (y + rect.height > vh) y = vh - rect.height - EDGE_MARGIN
  if (x < EDGE_MARGIN) x = EDGE_MARGIN
  if (y < EDGE_MARGIN) y = EDGE_MARGIN
  return { x, y }
}

/**
 * State for a fixed-position context menu. `menuEl` is the template ref
 * of the rendered menu; `initial` supplies any extra fields callers keep
 * on the menu state (e.g. the right-clicked file).
 */
export function useContextMenu(menuEl, initial = {}) {
  const contextMenu = ref({ show: false, x: 0, y: 0, ...initial })

  async function openContextMenu(event, extra = {}) {
    contextMenu.value = { ...initial, ...extra, show: true, x: event.clientX, y: event.clientY }
    // Wait for DOM render, then adjust position if off-screen
    await nextTick()
    const el = menuEl.value
    if (!el) return
    const { x, y } = clampToViewport(
      contextMenu.value.x,
      contextMenu.value.y,
      el.getBoundingClientRect(),
      window.innerWidth,
      window.innerHeight,
    )
    contextMenu.value.x = x
    contextMenu.value.y = y
  }

  function closeContextMenu() {
    contextMenu.value.show = false
  }

  return { contextMenu, openContextMenu, closeContextMenu }
}
