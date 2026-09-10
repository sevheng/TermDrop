import { listen } from '@tauri-apps/api/event'
import { onUnmounted, getCurrentInstance } from 'vue'

/**
 * Owns a set of Tauri event listeners and disposes them together.
 *
 * `listen(event, handler)` mirrors @tauri-apps/api/event but closes the
 * window where the component unmounts (or the group is disposed) while the
 * subscription is still being set up: the late-arriving unlisten is called
 * immediately instead of leaking. Groups are reusable after dispose().
 * Inside a component, the group is disposed on unmount automatically.
 */
export function useListenerGroup() {
  let unlisteners = []
  let generation = 0

  async function on(event, handler) {
    const gen = generation
    const unlisten = await listen(event, handler)
    if (gen !== generation) {
      unlisten()
      return
    }
    unlisteners.push(unlisten)
  }

  function dispose() {
    generation++
    const current = unlisteners
    unlisteners = []
    for (const unlisten of current) unlisten()
  }

  if (getCurrentInstance()) onUnmounted(dispose)

  return { listen: on, dispose }
}
