import { describe, it, expect } from 'vitest'
import {
  TAB_KIND,
  tabKind,
  isSshTab,
  tabsOfKind,
  hasRightPanel,
  acceptsCommands,
  closeActionFor,
} from '../tabKinds.js'

describe('tabKind', () => {
  it('treats a tab with no type as an SSH tab', () => {
    // SSH tabs predate the field and never carry one.
    expect(tabKind({ id: 'a' })).toBe(TAB_KIND.SSH)
    expect(tabKind({ id: 'a', type: undefined })).toBe(TAB_KIND.SSH)
    expect(isSshTab({ id: 'a' })).toBe(true)
  })

  it('reads the type when there is one', () => {
    expect(tabKind({ type: 'mongodb' })).toBe(TAB_KIND.MONGODB)
    expect(tabKind({ type: 'redis' })).toBe(TAB_KIND.REDIS)
  })

  it('falls back to SSH for an unknown type rather than throwing', () => {
    // A tab persisted by a future version must not break the tab strip.
    expect(tabKind({ type: 'something-new' })).toBe(TAB_KIND.SSH)
    expect(tabKind(null)).toBe(TAB_KIND.SSH)
  })
})

describe('tabsOfKind', () => {
  const tabs = [
    { id: '1' },
    { id: '2', type: 'mongodb' },
    { id: '3', type: 'redis' },
    { id: '4' },
  ]

  it('partitions the tab list', () => {
    expect(tabsOfKind(tabs, TAB_KIND.SSH).map(t => t.id)).toEqual(['1', '4'])
    expect(tabsOfKind(tabs, TAB_KIND.MONGODB).map(t => t.id)).toEqual(['2'])
    expect(tabsOfKind(tabs, TAB_KIND.REDIS).map(t => t.id)).toEqual(['3'])
  })

  it('handles a missing list', () => {
    expect(tabsOfKind(undefined, TAB_KIND.SSH)).toEqual([])
  })
})

describe('hasRightPanel', () => {
  it('is true only for shell tabs', () => {
    // The regression this guards: with a `!== "mongodb"` check, a Redis tab
    // would render the SFTP/Docker/Security panel beside it.
    expect(hasRightPanel({ id: '1' })).toBe(true)
    expect(hasRightPanel({ type: 'mongodb' })).toBe(false)
    expect(hasRightPanel({ type: 'redis' })).toBe(false)
    expect(hasRightPanel(null)).toBe(false)
  })
})

describe('acceptsCommands', () => {
  it('is true only for shell tabs', () => {
    // The security panel types remediation commands into a live root shell;
    // a datastore tab must never be a target.
    expect(acceptsCommands({ id: '1' })).toBe(true)
    expect(acceptsCommands({ type: 'mongodb' })).toBe(false)
    expect(acceptsCommands({ type: 'redis' })).toBe(false)
    expect(acceptsCommands(null)).toBe(false)
  })
})

describe('closeActionFor', () => {
  it('routes each kind to its store action', () => {
    expect(closeActionFor({ id: '1' })).toBe('disconnect')
    expect(closeActionFor({ type: 'mongodb' })).toBe('closeServiceTab')
    expect(closeActionFor({ type: 'redis' })).toBe('closeServiceTab')
  })
})
