import { describe, it, expect } from 'vitest'
import { useConfirmDialog } from '../useConfirmDialog.js'

describe('useConfirmDialog', () => {
  it('starts hidden', () => {
    const { confirmDialog } = useConfirmDialog()
    expect(confirmDialog.value.show).toBe(false)
  })

  it('applies defaults for title, message, and danger', () => {
    const { confirmDialog, openConfirm } = useConfirmDialog()
    openConfirm({ onConfirm: () => {} })
    expect(confirmDialog.value).toMatchObject({ show: true, title: 'Confirm', message: '', danger: false })
  })

  it('closes before running onConfirm', () => {
    const { confirmDialog, openConfirm } = useConfirmDialog()
    const seen = []
    openConfirm({
      title: 'Delete',
      message: 'Sure?',
      danger: true,
      onConfirm: () => seen.push(confirmDialog.value.show),
    })
    expect(confirmDialog.value).toMatchObject({ title: 'Delete', message: 'Sure?', danger: true })
    confirmDialog.value.onConfirm()
    expect(seen).toEqual([false])
  })
})
