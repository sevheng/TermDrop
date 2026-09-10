/**
 * The global PromptDialog, driven by window events so code outside a component
 * (the store, composables) can ask the user for a value.
 */
export function showPromptDialog(title, message, placeholder = '', type = 'text') {
  return new Promise((resolve) => {
    const responseHandler = (event) => {
      window.removeEventListener('prompt-dialog-response', responseHandler)
      resolve(event.detail)
    }
    window.addEventListener('prompt-dialog-response', responseHandler)
    window.dispatchEvent(new CustomEvent('prompt-dialog-open', {
      detail: { title, message, placeholder, type },
    }))
  })
}
