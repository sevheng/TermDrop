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
    }
  }

  return { confirmDialog, openConfirm }
}
