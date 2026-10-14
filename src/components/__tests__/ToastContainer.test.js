import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'

vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

import ToastContainer from '../ToastContainer.vue'

function emit(message, type = 'info') {
  window.dispatchEvent(new CustomEvent('app-toast', { detail: { message, type } }))
}

describe('ToastContainer', () => {
  let wrapper

  beforeEach(() => {
    vi.useFakeTimers()
    wrapper = mount(ToastContainer)
  })

  afterEach(() => {
    wrapper.unmount()
    vi.useRealTimers()
  })

  it('counts a repeated toast instead of stacking it', async () => {
    // A remote desktop rings its bell once per BEL character. Fifty of them
    // made fifty identical toasts, which filled the window and covered the
    // panel's toolbar.
    for (let i = 0; i < 50; i++) emit('kiosk is asking for attention')
    await nextTick()

    const toasts = wrapper.findAll('.pointer-events-auto')
    expect(toasts).toHaveLength(1)
    expect(toasts[0].text()).toContain('×50')
  })

  it('keeps a repeated toast up for as long as it keeps repeating', async () => {
    emit('bell')
    vi.advanceTimersByTime(3000)
    emit('bell')
    vi.advanceTimersByTime(3000)
    await nextTick()
    expect(wrapper.findAll('.pointer-events-auto')).toHaveLength(1)

    vi.advanceTimersByTime(1001)
    await nextTick()
    expect(wrapper.findAll('.pointer-events-auto')).toHaveLength(0)
  })

  it('still stacks toasts that differ in message or severity', async () => {
    emit('saved', 'success')
    emit('saved', 'error')
    emit('deleted', 'success')
    await nextTick()
    expect(wrapper.findAll('.pointer-events-auto')).toHaveLength(3)
    expect(wrapper.find('.tabular-nums').exists()).toBe(false)
  })
})
