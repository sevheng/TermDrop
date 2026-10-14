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

describe('SelectMenu sizing', () => {
  it('scales the trigger and the rows together', async () => {
    // The two used to be written separately at each call site's mercy; a
    // `md` trigger with `sm` rows reads as a control that has gone wrong.
    for (const [size, trigger, row] of [
      ['xs', 'text-2xs', 'text-2xs'],
      ['sm', 'text-xs', 'text-xs'],
      ['md', 'text-sm', 'text-sm'],
    ]) {
      const w = open({ size })
      expect(w.get('button').classes(), size).toContain(trigger)
      await w.get('button').trigger('click')
      await w.vm.$nextTick()
      expect(w.get('[role="option"]').classes(), size).toContain(row)
      w.unmount()
    }
  })

  it('fills its container only when asked to', async () => {
    // Every form-field usage passes `block`; without it the control collapses
    // to its label's width and sits cramped beside full-width inputs.
    const plain = open()
    expect(plain.get('button').classes()).not.toContain('w-full')
    plain.unmount()

    const blocky = open({ block: true })
    expect(blocky.get('button').classes()).toContain('w-full')
    blocky.unmount()
  })

  it('warns in development about a size it does not have', () => {
    // The validator's list is written out separately from SIZES because
    // defineProps is hoisted, so this is what keeps the two in step.
    const { validator } = SelectMenu.props.size
    expect(validator('md')).toBe(true)
    expect(validator('lg')).toBe(false)
  })
})
