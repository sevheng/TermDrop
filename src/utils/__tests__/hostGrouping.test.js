import { describe, it, expect } from 'vitest'
import {
  hostSearchText, filterHosts, groupHosts, groupNames, compareGroupNames,
} from '../hostGrouping.js'

const ssh = { id: 1, name: 'web-01', host: '10.0.1.4', username: 'deploy', group: 'prod' }
const mongo = { id: 2, name: 'orders', host: '', username: '', mongo_uri: 'mongodb://db.internal:27017' }
const redis = { id: 3, name: 'cache', host: '', username: '', redis_uri: 'redis://@10.0.9.9:6380/0' }

describe('hostSearchText', () => {
  it('includes the address of a datastore host', () => {
    // These have empty host/username columns, so before this they were
    // findable only by name.
    expect(hostSearchText(mongo)).toContain('db.internal')
    expect(hostSearchText(redis)).toContain('10.0.9.9')
  })

  it('never exposes a stored password to the search', () => {
    // Matching against the raw URI would let a password be found by typing
    // it — and confirm it by the match.
    const withSecret = { name: 'x', redis_uri: 'redis://:hunter2@h:6379' }
    expect(hostSearchText(withSecret)).not.toContain('hunter2')
  })
})

describe('filterHosts', () => {
  const hosts = [ssh, mongo, redis]

  it('returns everything for an empty query', () => {
    expect(filterHosts(hosts, '')).toHaveLength(3)
    expect(filterHosts(hosts, '   ')).toHaveLength(3)
  })

  it('finds a datastore host by its address', () => {
    expect(filterHosts(hosts, '10.0.9.9')).toEqual([redis])
    expect(filterHosts(hosts, 'db.internal')).toEqual([mongo])
  })

  it('still finds hosts by name, user, address and group', () => {
    expect(filterHosts(hosts, 'web')).toEqual([ssh])
    expect(filterHosts(hosts, 'deploy')).toEqual([ssh])
    expect(filterHosts(hosts, 'prod')).toEqual([ssh])
    expect(filterHosts(hosts, '10.0.1.4')).toEqual([ssh])
  })

  it('ignores case', () => {
    expect(filterHosts(hosts, 'WEB-01')).toEqual([ssh])
  })

  it('handles missing input', () => {
    expect(filterHosts(undefined, 'x')).toEqual([])
    expect(filterHosts(hosts, undefined)).toHaveLength(3)
  })
})

describe('groupHosts', () => {
  it('preserves the comparator order for numeric group names', () => {
    // The bug this replaces: v-for over an object hoists integer-like keys
    // into ascending numeric order, so "2" and "10" rendered as 2, 10 while
    // the comparator had asked for 10, 2.
    const hosts = [
      { id: 1, group: '10' }, { id: 2, group: '2' }, { id: 3, group: 'web' },
    ]
    expect(groupHosts(hosts).map(g => g.name)).toEqual(['10', '2', 'web'])
  })

  it('puts Ungrouped last', () => {
    const hosts = [{ id: 1, group: '' }, { id: 2, group: 'web' }]
    expect(groupHosts(hosts).map(g => g.name)).toEqual(['web', ''])
  })

  it('keeps every host in exactly one group', () => {
    const hosts = [{ id: 1, group: 'a' }, { id: 2, group: 'a' }, { id: 3, group: 'b' }]
    const groups = groupHosts(hosts)
    expect(groups.flatMap(g => g.hosts).map(h => h.id).sort()).toEqual([1, 2, 3])
    expect(groups.find(g => g.name === 'a').hosts).toHaveLength(2)
  })

  it('shows a created-but-empty group', () => {
    const groups = groupHosts([{ id: 1, group: 'web' }], ['staging'])
    expect(groups.map(g => g.name)).toEqual(['staging', 'web'])
    expect(groups.find(g => g.name === 'staging').hosts).toEqual([])
  })

  it('handles no hosts at all', () => {
    expect(groupHosts([])).toEqual([])
    expect(groupHosts(undefined)).toEqual([])
  })
})

describe('groupNames / compareGroupNames', () => {
  it('lists every name with Ungrouped last', () => {
    expect(groupNames([{ group: 'b' }, { group: '' }, { group: 'a' }])).toEqual(['a', 'b', ''])
  })

  it('sorts Ungrouped last whichever side it is on', () => {
    expect(compareGroupNames('', 'a')).toBe(1)
    expect(compareGroupNames('a', '')).toBe(-1)
  })
})
