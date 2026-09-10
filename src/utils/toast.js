/**
 * Show a toast through the global ToastContainer.
 * `type` is required ('success' | 'error' | 'warning' | 'info') so every
 * call site states its severity explicitly.
 */
export function toast(message, type) {
  if (!type) throw new Error('toast() requires a type')
  window.dispatchEvent(new CustomEvent('app-toast', { detail: { message, type } }))
}
