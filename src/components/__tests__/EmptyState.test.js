import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import { Shield } from 'lucide-vue-next'
import EmptyState from '../EmptyState.vue'

describe('EmptyState', () => {
  it('renders a spinner and reports busy while loading', () => {
    const w = mount(EmptyState, { props: { state: 'loading' } })
    expect(w.find('.animate-spin').exists()).toBe(true)
    expect(w.attributes('aria-busy')).toBe('true')
    expect(w.text()).toContain('Loading')
  })

  it('says something different for empty than for filtered', () => {
    // The distinction the old states mostly collapsed: "add something" and
    // "your filter is too narrow" need different responses from the reader.
    const empty = mount(EmptyState, { props: { state: 'empty' } })
    const filtered = mount(EmptyState, { props: { state: 'filtered' } })
    expect(empty.text()).not.toBe(filtered.text())
    expect(empty.text()).toContain('Nothing here yet')
    expect(filtered.text()).toContain('Nothing matches')
  })

  it('prefers a supplied title and hint over the defaults', () => {
    const w = mount(EmptyState, {
      props: { state: 'empty', title: 'No port forwards', hint: 'Add one to start' },
    })
    expect(w.text()).toContain('No port forwards')
    expect(w.text()).toContain('Add one to start')
    expect(w.text()).not.toContain('Nothing here yet')
  })

  it('emits action when the button is pressed', async () => {
    const w = mount(EmptyState, { props: { state: 'empty', actionLabel: 'Run audit' } })
    await w.get('button').trigger('click')
    expect(w.emitted('action')).toHaveLength(1)
  })

  it('disables the action and swaps the label while pending', () => {
    const w = mount(EmptyState, {
      props: { state: 'empty', actionLabel: 'Install', actionPending: true, pendingLabel: 'Installing…' },
    })
    expect(w.get('button').attributes('disabled')).toBeDefined()
    expect(w.text()).toContain('Installing…')
  })

  it('renders no button at all when no action is offered', () => {
    expect(mount(EmptyState, { props: { state: 'empty' } }).find('button').exists()).toBe(false)
  })

  it('takes a custom icon and keeps the slot content', () => {
    const w = mount(EmptyState, {
      props: { state: 'empty', icon: Shield },
      slots: { default: '<code>sudo systemctl start docker</code>' },
    })
    // Docker's states carry a command block; a thinner API would lose it.
    expect(w.html()).toContain('sudo systemctl start docker')
  })

  it('tints an error state', () => {
    const w = mount(EmptyState, { props: { state: 'error', title: 'Could not read the directory' } })
    expect(w.html()).toContain('text-bad')
  })

  it('refuses a state it does not know', () => {
    const { validator } = EmptyState.props.state
    expect(validator('empty')).toBe(true)
    expect(validator('nonsense')).toBe(false)
  })
})
