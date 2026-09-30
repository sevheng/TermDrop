import { describe, it, expect } from 'vitest'
import { jumpHostCandidates, sshHostOptions } from '../hostOptions.js'

const ssh = (id, over = {}) => ({ id, name: `h${id}`, host: `10.0.0.${id}`, username: 'u', ...over })

describe('sshHostOptions', () => {
  it('offers a direct connection first, then each host with its address', () => {
    expect(sshHostOptions([ssh(1)])).toEqual([
      { value: '', label: 'Connect directly' },
      { value: 1, label: 'h1', hint: 'u@10.0.0.1' },
    ])
  })
})

describe('jumpHostCandidates', () => {
  it('offers other SSH hosts', () => {
    expect(jumpHostCandidates([ssh(1), ssh(2)], 1).map(h => h.id)).toEqual([2])
  })

  it('offers every SSH host while a new host is being added', () => {
    expect(jumpHostCandidates([ssh(1), ssh(2)], null).map(h => h.id)).toEqual([1, 2])
  })

  it('never offers a datastore row', () => {
    const redis = { id: 3, name: 'cache', host: '', redis_uri: 'redis://x' }
    const mongo = { id: 4, name: 'db', host: '', mongo_uri: 'mongodb://x' }
    expect(jumpHostCandidates([redis, mongo, ssh(2)], 1).map(h => h.id)).toEqual([2])
  })

  it('never offers a host that has its own jump host', () => {
    // One hop only; the backend would refuse the chain at connect time.
    expect(jumpHostCandidates([ssh(2, { jump_host_id: 9 }), ssh(3)], 1).map(h => h.id)).toEqual([3])
  })
})
