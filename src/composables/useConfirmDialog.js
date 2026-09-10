import { ref } from 'vue'

/**
 * State for a <ConfirmDialog>. `openConfirm({ title, message, danger, onConfirm })`
 * shows it; the dialog closes itself before running `onConfirm`.
 */
export function useConfirmDialog() {
  const confirmDialog = ref({
    show: false,
    title: '',
    message: '',
    danger: false,
    onConfirm: () => {},
    onCancel: () => {},
  })

  function openConfirm(options) {
    confirmDialog.value = {
      show: true,
      title: options.title || 'Confirm',
      message: options.message || '',
      danger: options.danger || false,
      onConfirm: () => {
        confirmDialog.value.show = false
        options.onConfirm()
      },
      // Optional: callers that need to know the user declined, such as a
      // discard-changes prompt guarding a navigation.
      onCancel: () => {
        confirmDialog.value.show = false
        options.onCancel?.()
      },
    }
  }

  return { confirmDialog, openConfirm }
}
