import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import ModalShell from '../ModalShell.vue'

const withFields = {
  props: { show: true },
  slots: { default: '<input id="a" /><button id="b">Save</button>' },
  attachTo: document.body,
}

describe('ModalShell focus trap', () => {
  it('marks itself as a dialog for assistive tech', () => {
    const w = mount(ModalShell, { props: { show: true } })
    const panel = w.get('[role="dialog"]')
    expect(panel.attributes('aria-modal')).toBe('true')
  })

  it('emits close on Escape', async () => {
    const w = mount(ModalShell, { props: { show: true } })
    await w.get('.fixed').trigger('keydown', { key: 'Escape' })
    expect(w.emitted('close')).toHaveLength(1)
  })

  it('leaves Escape alone when the dialog opts out', async () => {
    // A few dialogs are dismissed only by their own buttons.
    const w = mount(ModalShell, { props: { show: true, closeOnEscape: false } })
    await w.get('.fixed').trigger('keydown', { key: 'Escape' })
    expect(w.emitted('close')).toBeUndefined()
  })

  it('keeps Tab inside the dialog', async () => {
    // Without this, Tab walked into the application behind the scrim, where
    // the controls are covered and cannot be seen.
    const w = mount(ModalShell, withFields)
    await w.vm.$nextTick()
    const input = document.getElementById('a')
    const button = document.getElementById('b')

    input.focus()
    await w.get('.fixed').trigger('keydown', { key: 'Tab' })
    expect(document.activeElement).toBe(button)

    // ...and wraps at the end, because a dialog is a closed loop.
    await w.get('.fixed').trigger('keydown', { key: 'Tab' })
    expect(document.activeElement).toBe(input)
    w.unmount()
  })

  it('walks backwards on Shift+Tab', async () => {
    const w = mount(ModalShell, withFields)
    await w.vm.$nextTick()
    document.getElementById('a').focus()
    await w.get('.fixed').trigger('keydown', { key: 'Tab', shiftKey: true })
    expect(document.activeElement).toBe(document.getElementById('b'))
    w.unmount()
  })

  it('focuses the first field when it opens', async () => {
    const w = mount(ModalShell, { ...withFields, props: { show: false } })
    await w.setProps({ show: true })
    await w.vm.$nextTick()
    await w.vm.$nextTick()
    expect(document.activeElement).toBe(document.getElementById('a'))
    w.unmount()
  })

  it('gives focus back to whatever opened it', async () => {
    const opener = document.createElement('button')
    document.body.appendChild(opener)
    opener.focus()

    const w = mount(ModalShell, { ...withFields, props: { show: false } })
    await w.setProps({ show: true })
    await w.vm.$nextTick()
    await w.setProps({ show: false })
    await w.vm.$nextTick()

    expect(document.activeElement).toBe(opener)
    w.unmount()
    opener.remove()
  })

  it('does not trap into nothing when the dialog has no controls', async () => {
    const w = mount(ModalShell, { props: { show: true }, slots: { default: 'just text' } })
    await w.get('.fixed').trigger('keydown', { key: 'Tab' })
    expect(w.emitted('close')).toBeUndefined()
  })
})
