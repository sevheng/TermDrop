/**
 * Dismissal of the launch splash that `index.html` paints inline.
 *
 * The splash exists because the window is on screen long before the bundle
 * is; this is the other end of that, and its whole job is to get out of the
 * way at the right moment.
 */

const FADE_MS = 200

/**
 * How long to wait for real content before showing the app anyway.
 *
 * Load-bearing: a hung or failed `get_hosts`/`get_setting` must degrade to a
 * usable app, never to a permanent splash.
 */
const CAP_MS = 1500

/**
 * Fade the splash out once the app has content underneath it.
 *
 * Waiting for `ready` is what stops the splash fading into an empty sidebar
 * that then pops -- the same defect one layer in. Only the splash animates:
 * any opacity on `#app` would make it a containing block for the app's many
 * `position: fixed` menus and modals.
 */
export function dismissSplash(ready) {
  const el = document.getElementById('td-splash')
  if (!el) return

  const capped = Promise.race([ready, new Promise(r => setTimeout(r, CAP_MS))])

  capped.then(() => {
    // Two frames, not one: the first runs before the frame carrying the newly
    // loaded hosts has painted, so waiting for the second guarantees the app
    // is really on screen before the splash starts to uncover it.
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        performance.mark('td-ready')
        logStartupPhase('td-ready')
        el.classList.add('td-splash-out')
        // A timer rather than `transitionend`: under prefers-reduced-motion the
        // transition is `none`, and then that event never fires at all.
        setTimeout(() => el.remove(), FADE_MS + 50)
      })
    )
  })
}

/**
 * Log one startup phase, measured from the mark `index.html` sets before
 * anything else runs. Dev only -- the marks themselves are free, so they are
 * always taken, but a release console stays clean.
 */
export function logStartupPhase(mark) {
  if (!import.meta.env.DEV) return
  try {
    const m = performance.measure(`td-html:${mark}`, 'td-html', mark)
    console.info(`[startup] html -> ${mark}: ${Math.round(m.duration)}ms`)
  } catch {
    // A missing mark is not worth reporting over.
  }
}
