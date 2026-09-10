import { ref } from 'vue'
import { terminalTheme } from '../themes/index.js'

/**
 * Which palette the app is wearing.
 *
 * The whole switch is one class on <html>: tokens.css defines light on bare
 * :root and dark under html.dark, and every component reads tokens, so nothing
 * else in the app needs a `dark:` variant or even needs to know.
 *
 * Kept as a module-level ref rather than a store field so the terminal and the
 * settings panel can read it without importing the store into either.
 */

export const THEMES = ['dark', 'light']
// Read by the inline script in index.html before first paint. Kept in sync
// here; nothing else in the app should read it, because SQLite is the record.
const STORAGE_KEY = 'td-theme'

export const theme = ref('dark')

/** Put the class on <html> and remember the choice. */
export function applyTheme(next) {
  const value = THEMES.includes(next) ? next : 'dark'
  theme.value = value
  document.documentElement.classList.toggle('dark', value === 'dark')
  // Written here as well as in settings so the very first paint after a
  // restart is the right theme, before the async settings load returns.
  try {
    localStorage.setItem(STORAGE_KEY, value)
  } catch {
    // Private mode or blocked storage: the setting in SQLite is the record
    // that matters, this is only to avoid a flash of the wrong theme.
  }
}

/** The xterm palette matching the current theme. */
export function currentTerminalTheme() {
  return terminalTheme(theme.value)
}
