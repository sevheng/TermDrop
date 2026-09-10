import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import SelectMenu from '../SelectMenu.vue'

const options = Array.from({ length: 30 }, (_, i) => ({ value: i, label: `Option ${i}` }))

function open(props = {}) {
  const w = mount(SelectMenu, {
    props: { modelValue: 20, options, ...props },
    attachTo: document.body,
  })
  return w
}

describe('SelectMenu scrolling', () => {
  it('does not reposition itself when the list is the thing scrolling', async () => {
    // The bug: the scroll listener is capturing, so it also saw the menu's own
    // scrolling, and place() ended by scrolling the selection back into view —
    // so the list could not be scrolled at all.
    const w = open()
    await w.get('button').trigger('click')
    await w.vm.$nextTick()

    const list = w.get('[role="listbox"]').element
    const before = { ...w.vm.pos }

    list.dispatchEvent(new Event('scroll', { bubbles: true }))
    await w.vm.$nextTick()

    // Position untouched, and nothing yanked the list back.
    expect(w.vm.pos.top).toBe(before.top)
    w.unmount()
  })

  it('still repositions when something else scrolls', async () => {
    // A panel scrolling under a fixed menu must still move it; the guard has
    // to exclude only the menu's own scrolling, not all of it.
    const w = open()
    await w.get('button').trigger('click')
    await w.vm.$nextTick()

    const outside = document.createElement('div')
    document.body.appendChild(outside)
    const before = w.vm.pos

    outside.dispatchEvent(new Event('scroll', { bubbles: true }))
    await w.vm.$nextTick()

    // place() reassigns pos, so a new object means it ran.
    expect(w.vm.pos).not.toBe(before)
    outside.remove()
    w.unmount()
  })

  it('gives the list its own scrollbar and a capped height', async () => {
    const w = open()
    await w.get('button').trigger('click')
    await w.vm.$nextTick()
    const list = w.get('[role="listbox"]')
    expect(list.classes()).toContain('overflow-y-auto')
    expect(w.vm.pos.maxHeight).toBeGreaterThan(0)
    expect(w.vm.pos.maxHeight).toBeLessThanOrEqual(288)
    w.unmount()
  })
})
