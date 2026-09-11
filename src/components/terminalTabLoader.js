/**
 * The one place that names the TerminalTab chunk.
 *
 * `TerminalTab` pulls in xterm, which is ~400 KB -- and no terminal tab is
 * open at launch, so a static import put all of it on the path to the first
 * frame for nothing. It is loaded lazily instead, and warmed while the app is
 * idle so the first tab does not pay for the fetch.
 *
 * Both the async component and the warm-up go through this module so they
 * share one import specifier, which is what makes Vite emit a single chunk and
 * the module registry hand back a single promise.
 *
 * It deliberately imports nothing itself: a static import of TerminalTab here
 * would put xterm straight back into the boot graph via that edge.
 */
export const loadTerminalTab = () => import('./TerminalTab.vue')
