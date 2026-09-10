import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import { Layers } from 'lucide-vue-next'
import PanelHeader from '../PanelHeader.vue'

describe('PanelHeader', () => {
  it('tints the strip so a panel reads as a panel', () => {
    expect(mount(PanelHeader, { props: { title: 'Docker' } }).classes()).toContain('bg-surface')
  })

  it('has one height per slot, not five', () => {
    expect(mount(PanelHeader, { props: { title: 'X' } }).classes()).toContain('h-9')
    expect(mount(PanelHeader, { props: { title: 'X', dense: true } }).classes()).toContain('h-7')
  })

  it('gives a truncating subtitle its full text on hover', () => {
    const w = mount(PanelHeader, {
      props: { title: 'Local Redis', subtitle: 'redis://***@localhost:6380/0', subtitleMono: true },
    })
    expect(w.html()).toContain('font-mono')
    expect(w.get('[title]').attributes('title')).toContain('localhost:6380')
  })

  it('renders meta as tabular so counts do not jitter', () => {
    const w = mount(PanelHeader, { props: { title: 'Keys', meta: '21 keys' } })
    expect(w.html()).toContain('tabular-nums')
  })

  it('takes a brand colour for the datastore panels', () => {
    const w = mount(PanelHeader, { props: { title: 'Redis', icon: Layers, iconClass: 'text-redis' } })
    expect(w.html()).toContain('text-redis')
  })

  it('renders the badges and actions slots', () => {
    const w = mount(PanelHeader, {
      props: { title: 'Redis' },
      slots: { badges: '<span>via jump</span>', actions: '<button>Back up</button>' },
    })
    expect(w.text()).toContain('via jump')
    expect(w.find('button').exists()).toBe(true)
  })
})
