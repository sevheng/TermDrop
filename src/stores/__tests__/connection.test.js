import { describe, it, expect, vi, beforeEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'

/**
 * The store's first tests.
 *
 * It reaches outside through a handful of modules and nothing else, so mocking
 * those four is the whole harness. `listen` is captured rather than stubbed
 * away, because the captured handlers are the only way to drive the `ssh-*`
 * events the store routes.
 */
const invokes = []
let invokeImpl = () => Promise.resolve(null)
vi.mock('../../utils/invoke.js', () => ({
  invokeWithSlowWarning: (command, args) => {
    invokes.push([command, args])
    return invokeImpl(command, args)
  },
  invoke: (command, args) => {
    invokes.push([command, args])
    return invokeImpl(command, args)
  },
}))

const toasts = []
vi.mock('../../utils/toast.js', () => ({
  toast: (message, type) => toasts.push([message, type]),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: () => Promise.resolve(() => {}),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }))
vi.mock('../../composables/usePromptDialog.js', () => ({
  showPromptDialog: vi.fn(() => Promise.resolve(null)),
}))
vi.mock('../../composables/useTheme.js', () => ({ applyTheme: vi.fn() }))

const { useConnectionStore } = await import('../connection.js')

function storeWithHost(host = { id: 1, name: 'web-01', host: '10.0.0.1', auth_type: 'password' }) {
  const store = useConnectionStore()
  store.hosts = [host]
  return store
}

beforeEach(() => {
  setActivePinia(createPinia())
  invokes.length = 0
  toasts.length = 0
  invokeImpl = () => Promise.resolve(null)
})

describe('attachSftp', () => {
  it('settles the tab even when SFTP is unavailable', async () => {
    // The bug: `connecting` was cleared only on the success path, so a host
    // with the sftp subsystem disabled -- ordinary on a hardened server, near
    // universal on a network appliance -- left the tab spinning forever. The
    // shell worked fine, which is what made it so confusing.
    invokeImpl = command => {
      if (command === 'ssh_connect') return Promise.resolve('session-1')
      if (command === 'sftp_connect') return Promise.reject('subsystem request failed')
      return Promise.resolve(null)
    }

    const store = storeWithHost()
    await store.connect(1)

    expect(store.tabs).toHaveLength(1)
    expect(store.tabs[0].connecting).toBe(false)
    expect(store.tabs[0].sftpSessionId).toBe(null)
    expect(store.connectingHostId).toBe(null)
  })

  it('warns rather than failing the connection', async () => {
    // The terminal is the point; SFTP is a side channel. Losing it must not
    // take the tab with it.
    invokeImpl = command => {
      if (command === 'ssh_connect') return Promise.resolve('session-1')
      if (command === 'sftp_connect') return Promise.reject('nope')
      return Promise.resolve(null)
    }

    const store = storeWithHost()
    await expect(store.connect(1)).resolves.toBe('session-1')
    expect(toasts.some(([m, t]) => m.includes('SFTP') && t === 'warning')).toBe(true)
  })

  it('records the session id when SFTP does connect', async () => {
    invokeImpl = command => {
      if (command === 'ssh_connect') return Promise.resolve('session-1')
      if (command === 'sftp_connect') return Promise.resolve('sftp-9')
      return Promise.resolve(null)
    }

    const store = storeWithHost()
    await store.connect(1)

    expect(store.tabs[0].sftpSessionId).toBe('sftp-9')
    expect(store.tabs[0].connecting).toBe(false)
  })

  it('passes the password to SFTP only for password auth', async () => {
    // A key-auth host has no password to forward, and sending one would be
    // both pointless and a credential in a place it does not belong.
    invokeImpl = () => Promise.resolve('id')

    const store = storeWithHost({ id: 1, name: 'k', host: 'h', auth_type: 'key' })
    await store.connect(1, 'secret')

    const [, args] = invokes.find(([c]) => c === 'sftp_connect')
    expect(args.password).toBeUndefined()
  })
})
