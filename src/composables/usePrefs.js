import { ref } from 'vue'
import { invoke } from '../utils/invoke.js'

/**
 * Layout and list preferences: panel widths, SFTP columns, the host list's
 * view mode, its collapsed groups and the empty groups the user created.
 *
 * SQLite's settings table is the record. These used to live only in
 * localStorage, which a webview reset wipes and which a native frontend cannot
 * read at all (docs/gpui-migration-plan.md, Phase 1a). localStorage stays as a
 * synchronous cache, so the first paint already has the right widths, and
 * `loadPrefs` reconciles the two under the launch splash. A value found only
 * in localStorage comes from an install that predates this, and is copied into
 * SQLite once.
 *
 * Each preference is one shared ref, so every component reading it sees the
 * value SQLite returns. Nothing is saved implicitly: a width changes on every
 * mousemove of a drag, so callers save with `savePref` when they are done.
 */

const json = { parse: JSON.parse, stringify: JSON.stringify }
const int = {
  parse: (raw) => {
    const n = parseInt(raw, 10)
    if (Number.isNaN(n)) throw new Error(`not a number: ${raw}`)
    return n
  },
  stringify: String,
}
const text = { parse: (raw) => raw, stringify: String }

export const PREFS = {
  'sidebar-width': int,
  'sftp-width': int,
  'sftp-columns': json,
  'host-view-mode': text,
  'host-collapsed-groups': json,
  'host-custom-groups': json,
}

const refs = new Map()
// The raw value SQLite is known to hold, so an unchanged save costs no IPC.
const stored = new Map()

function decode(key, raw) {
  if (raw == null) return undefined
  try {
    return PREFS[key].parse(raw)
  } catch {
    return undefined
  }
}

function readCache(key) {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function writeCache(key, raw) {
  try {
    localStorage.setItem(key, raw)
  } catch {
    // Blocked storage only costs the first paint; SQLite still has it.
  }
}

/** The shared ref for a preference, seeded from the cache or `fallback`. */
export function usePref(key, fallback) {
  if (!(key in PREFS)) throw new Error(`unknown preference: ${key}`)
  if (!refs.has(key)) {
    const cached = decode(key, readCache(key))
    refs.set(key, ref(cached === undefined ? fallback : cached))
  }
  return refs.get(key)
}

/** Persist the preference's current value. */
export async function savePref(key) {
  const r = refs.get(key)
  if (!r) return
  const raw = PREFS[key].stringify(r.value)
  writeCache(key, raw)
  if (stored.get(key) === raw) return
  stored.set(key, raw)
  try {
    await invoke('set_setting', { key, value: raw })
  } catch (err) {
    stored.delete(key)
    console.error(`[prefs] saving ${key} failed:`, err)
  }
}

/**
 * Reconcile every preference with SQLite. A value there wins and refreshes the
 * cache; a value only in the cache is migrated into SQLite.
 */
export async function loadPrefs() {
  await Promise.all(Object.keys(PREFS).map(async (key) => {
    const raw = await invoke('get_setting', { key })
    if (raw != null) {
      stored.set(key, raw)
      const value = decode(key, raw)
      if (value === undefined) return
      writeCache(key, raw)
      if (refs.has(key)) refs.get(key).value = value
      return
    }
    const cached = readCache(key)
    if (cached != null && decode(key, cached) !== undefined) {
      await invoke('set_setting', { key, value: cached })
      stored.set(key, cached)
    }
  }))
}

/** Test hook: forget every ref and what SQLite was known to hold. */
export function resetPrefsForTest() {
  refs.clear()
  stored.clear()
}
