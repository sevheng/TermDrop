import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import { Star } from 'lucide-vue-next'
import IconButton from '../IconButton.vue'

const base = { icon: Star, label: 'Favourite' }

describe('IconButton', () => {
  it('is labelled for both pointer and screen reader', () => {
    // The old buttons carried a bare `title`, so assistive tech got nothing.
    const w = mount(IconButton, { props: base })
    expect(w.attributes('title')).toBe('Favourite')
    expect(w.attributes('aria-label')).toBe('Favourite')
  })

  it('meets a usable hit target', () => {
    // Was p-0.5 around a 12px glyph — a 16px target.
    expect(mount(IconButton, { props: base }).classes()).toContain('h-6')
    expect(mount(IconButton, { props: { ...base, size: 'md' } }).classes()).toContain('h-7')
  })

  it('shows where the target is on hover', () => {
    expect(mount(IconButton, { props: base }).classes()).toContain('hover:bg-raised')
  })

  it('reports toggle state only when it is a toggle', () => {
    expect(mount(IconButton, { props: base }).attributes('aria-pressed')).toBeUndefined()
    expect(mount(IconButton, { props: { ...base, active: true } }).attributes('aria-pressed')).toBe('true')
    expect(mount(IconButton, { props: { ...base, active: false } }).attributes('aria-pressed')).toBe('false')
  })

  it('emits click, and is disabled while disabled or pending', async () => {
    const w = mount(IconButton, { props: base })
    await w.trigger('click')
    expect(w.emitted('click')).toHaveLength(1)
    for (const extra of [{ disabled: true }, { pending: true }]) {
      expect(mount(IconButton, { props: { ...base, ...extra } }).attributes('disabled')).toBeDefined()
    }
  })

  it('spins and reports busy while pending', () => {
    const w = mount(IconButton, { props: { ...base, pending: true } })
    expect(w.find('.animate-spin').exists()).toBe(true)
    expect(w.attributes('aria-busy')).toBe('true')
  })

  it('tints its hover by tone', () => {
    expect(mount(IconButton, { props: { ...base, tone: 'bad' } }).classes()).toContain('hover:text-bad')
    expect(mount(IconButton, { props: base }).classes()).toContain('hover:text-ink')
  })

  it('keeps the focus ring for keyboard users only', () => {
    // focus-visible, not focus: a ring on every mouse click is worse than none.
    const cls = mount(IconButton, { props: base }).classes().join(' ')
    expect(cls).toContain('focus-visible:ring-accent')
    expect(cls).not.toContain('focus:ring-accent')
  })
})
