/**
 * Limits shared with the backend.
 *
 * These mirror the constants in `src-tauri/src/redis.rs`. The backend clamps
 * whatever it is sent, so these are the values the UI asks for rather than a
 * second enforcement point — but they must not drift, or the UI will show a
 * page size it never actually gets.
 */

/** Mirrors `redis::MAX_KEYS_PER_PAGE`. */
export const MAX_KEYS_PER_PAGE = 200
/** Mirrors `redis::MAX_ELEMENTS_PER_PAGE`. */
export const MAX_ELEMENTS_PER_PAGE = 200
/** Mirrors `redis::MAX_DUMP_KEY_BYTES`. */
export const DEFAULT_MAX_DUMP_KEY_BYTES = 64 * 1024 * 1024
