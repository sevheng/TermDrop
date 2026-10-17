// Fake Tauri backend: renders TermDrop's real UI in a browser with demo data,
// for README screenshots. Every hostname and address is from a documentation
// range or a private network; nothing here is a real system.
(function () {
  const q = new URLSearchParams(location.search)
  const theme = q.get('theme') || 'dark'

  const callbacks = new Map()
  let nextId = 1
  const listeners = new Map() // event -> [{ id, handler }]
  const emit = (event, payload) => {
    for (const l of listeners.get(event) || []) callbacks.get(l.handler)?.({ event, id: l.id, payload })
  }

  let n = 1
  const host = (o) => Object.assign({
    id: n++, name: 'host', host: '', port: 22, username: 'deploy', auth_type: 'key', key_path: '~/.ssh/id_ed25519',
    group: null, favorite: 0, last_connected_at: null, created_at: '2026-01-01 00:00:00', mongo_uri: null,
    mongo_local_uri: null, redis_uri: null, redis_tunnel_host_id: null, jump_host_id: null,
    vnc_host: null, vnc_port: null, vnc_username: null, vnc_tunnel_host_id: null, vnc_view_only: 0,
  }, o)
  const hosts = [
    host({ name: 'api-prod-01', host: '10.20.0.11', group: 'Production', favorite: 1, last_connected_at: new Date(Date.now() - 36e5).toISOString() }),
    host({ name: 'api-prod-02', host: '10.20.0.12', group: 'Production' }),
    host({ name: 'worker-prod', host: '10.20.0.21', group: 'Production' }),
    host({ name: 'bastion', host: 'bastion.example.com', group: 'Production', favorite: 1, last_connected_at: new Date(Date.now() - 864e5).toISOString() }),
    host({ name: 'staging', host: 'staging.example.com', group: 'Staging' }),
    host({ name: 'ci-runner', host: '203.0.113.40', group: 'Staging', username: 'ci' }),
    host({ name: 'Orders DB', host: '', port: 0, username: '', auth_type: 'password', key_path: null, mongo_uri: 'mongodb+srv://cluster0.example.net', group: 'Data' }),
    host({ name: 'Session cache', host: '', port: 0, username: '', auth_type: 'password', key_path: null, redis_uri: 'redis://cache.internal:6379', group: 'Data' }),
    host({ name: 'Design iMac', host: '', port: 0, username: '', auth_type: 'password', key_path: null, vnc_host: '192.168.1.20', vnc_port: 5900, group: 'Office' }),
    host({ name: 'homelab', host: '192.168.1.10', group: 'Office', username: 'pi' }),
  ]

  // --- terminal session ---------------------------------------------------
  const E = '\x1b['
  const g = s => `${E}1;32m${s}${E}0m`, b = s => `${E}1;34m${s}${E}0m`, c = s => `${E}36m${s}${E}0m`
  const y = s => `${E}33m${s}${E}0m`, d = s => `${E}2m${s}${E}0m`, r = s => `${E}31m${s}${E}0m`
  const prompt = `${g('deploy@api-prod-01')}:${b('~')}$ `
  const session = [
    `${d('Welcome to Ubuntu 24.04.1 LTS (GNU/Linux 6.8.0-45-generic x86_64)')}\r\n\r\n`,
    `${d('Last login: Fri Oct 17 09:12:44 2026 from 10.20.0.4')}\r\n`,
    prompt, 'uptime\r\n',
    ' 14:32:07 up 12 days,  4:31,  2 users,  load average: 0.42, 0.38, 0.35\r\n',
    prompt, "docker ps --format 'table {{.Names}}\\t{{.Image}}\\t{{.Status}}'\r\n",
    `${E}1mNAMES     IMAGE                 STATUS${E}0m\r\n`,
    `api       ghcr.io/acme/api:2.14  ${g('Up 3 days (healthy)')}\r\n`,
    `worker    ghcr.io/acme/api:2.14  ${g('Up 3 days')}\r\n`,
    `nginx     nginx:1.27-alpine     ${g('Up 12 days')}\r\n`,
    `redis     redis:7.2             ${g('Up 12 days')}\r\n`,
    prompt, 'ls -la app\r\n',
    'total 48\r\n',
    `drwxr-xr-x  6 deploy deploy 4096 Oct 16 18:04 ${b('.')}\r\n`,
    `drwxr-x--- 12 deploy deploy 4096 Oct 17 09:12 ${b('..')}\r\n`,
    `-rw-r--r--  1 deploy deploy  412 Oct 16 18:04 .env.example\r\n`,
    `drwxr-xr-x  8 deploy deploy 4096 Oct 16 18:04 ${b('.git')}\r\n`,
    `-rw-r--r--  1 deploy deploy 1203 Oct 16 18:04 docker-compose.yml\r\n`,
    `-rwxr-xr-x  1 deploy deploy 1832 Oct 16 18:04 ${g('deploy.sh')}\r\n`,
    `drwxr-xr-x  3 deploy deploy 4096 Oct 16 18:04 ${b('migrations')}\r\n`,
    `drwxr-xr-x  9 deploy deploy 4096 Oct 16 18:04 ${b('src')}\r\n`,
    prompt, 'git -C app log --oneline -5\r\n',
    `${y('9f3c2a1')} ${c('(HEAD -> main, origin/main)')} feat(api): paginate order history\r\n`,
    `${y('4be81d0')} fix(worker): retry webhook delivery with backoff\r\n`,
    `${y('a71e5c9')} chore(deps): bump pg to 8.13\r\n`,
    `${y('03d9f44')} ${c('(tag: v2.14.0)')} release: 2.14.0\r\n`,
    `${y('e5b2c17')} perf(api): cache product lookups in redis\r\n`,
    prompt, 'free -h\r\n',
    `${E}1m               total        used        free      shared  buff/cache   available${E}0m\r\n`,
    'Mem:           7.8Gi       2.9Gi       1.1Gi        24Mi       3.8Gi       4.6Gi\r\n',
    'Swap:          2.0Gi          0B       2.0Gi\r\n',
    prompt, 'df -h / /var/lib/docker\r\n',
    `${E}1mFilesystem      Size  Used Avail Use% Mounted on${E}0m\r\n`,
    `/dev/sda1        80G   31G   49G  ${g('39%')} /\r\n`,
    `/dev/sdb1       200G  141G   59G  ${y('71%')} /var/lib/docker\r\n`,
    prompt, 'tail -n 4 /var/log/nginx/access.log\r\n',
    `203.0.113.7 - - [17/Oct/2026:14:31:58 +0000] "GET /api/orders?page=2 HTTP/2" ${g('200')} 5123\r\n`,
    `198.51.100.23 - - [17/Oct/2026:14:31:59 +0000] "POST /api/checkout HTTP/2" ${g('201')} 812\r\n`,
    `203.0.113.7 - - [17/Oct/2026:14:32:01 +0000] "GET /api/products/42 HTTP/2" ${g('200')} 2210\r\n`,
    `192.0.2.150 - - [17/Oct/2026:14:32:04 +0000] "GET /wp-login.php HTTP/1.1" ${r('404')} 153\r\n`,
    prompt,
  ]
  window.__demo = { emit, channels: [] }
  window.__demo.typeInto = async (text, delay = 45) => {
    const ch = window.__demo.channels.at(-1)
    for (const chr of text) { ch.send(chr); await new Promise(res => setTimeout(res, delay)) }
  }

  const stats = {
    load: '0.42', ram: '2.9/7.8GB (37%)', disk: '31G/80G (39%)', uptime: '12d 4h 31m',
    os: 'Ubuntu 24.04.1 LTS', kernel: '6.8.0-45-generic', arch: 'x86_64', cores: '4',
    netdev: 'lo: 1000 1000\neth0: 81234567890 23456789012',
  }
  const panel = {
    processes: [
      { pid: '2201', command: 'node dist/server.js', cpu: '12.4', mem: '6.1', uptime: '3-02:14:51' },
      { pid: '2288', command: 'node dist/worker.js', cpu: '4.7', mem: '3.8', uptime: '3-02:14:49' },
      { pid: '1102', command: 'redis-server *:6379', cpu: '1.2', mem: '1.9', uptime: '12-04:30:12' },
      { pid: '980', command: 'nginx: worker process', cpu: '0.6', mem: '0.4', uptime: '12-04:31:02' },
      { pid: '812', command: 'dockerd -H fd://', cpu: '0.4', mem: '1.1', uptime: '12-04:31:20' },
    ],
    network: {
      ports: [
        { proto: 'LISTEN', local: '0.0.0.0:22', state: '0', process: 'users:(("sshd",pid=800,fd=3))' },
        { proto: 'LISTEN', local: '0.0.0.0:443', state: '0', process: 'users:(("nginx",pid=975,fd=6))' },
        { proto: 'LISTEN', local: '127.0.0.1:6379', state: '0', process: 'users:(("redis-server",pid=1102,fd=6))' },
      ],
      interfaces: [{ name: 'eth0', rx_bytes: 81234567890, tx_bytes: 23456789012, rx_rate: 0, tx_rate: 0 }],
      established_count: 42,
    },
    disk: {
      mounts: [{ filesystem: '/dev/sda1', size: '80G', used: '31G', available: '49G', percent: '39%', mount: '/' }],
      dirs: [{ path: '/var/lib/docker', size: '18G' }, { path: '/var/log', size: '1.2G' }],
    },
  }

  const now = Math.floor(Date.now() / 1000)
  const file = (name, dir, size, ago) => ({
    name, path: '/home/deploy/' + name, size, is_dir: dir, modified: now - ago,
    permissions: dir ? 0o40755 : (name.endsWith('.sh') ? 0o100755 : 0o100644), uid: 1000, gid: 1000,
  })
  const files = [
    file('app', true, 4096, 3600 * 20), file('backups', true, 4096, 86400 * 3), file('logs', true, 4096, 600),
    file('.bashrc', false, 3771, 86400 * 90), file('deploy.sh', false, 1832, 3600 * 20),
    file('docker-compose.yml', false, 1203, 3600 * 20), file('nginx.conf', false, 2480, 86400 * 12),
    file('orders-2026-10.csv', false, 4823117, 86400), file('README.md', false, 2214, 86400 * 30),
  ]

  const containers = [
    { id: '3f9a1c2b7d4e', name: 'api', image: 'ghcr.io/acme/api:2.14', status: 'Up 3 days (healthy)', ports: '127.0.0.1:3000->3000/tcp', created: '2026-10-14 10:02:11', state: 'running', running: true },
    { id: '8b2e4f6a1c3d', name: 'worker', image: 'ghcr.io/acme/api:2.14', status: 'Up 3 days', ports: '', created: '2026-10-14 10:02:12', state: 'running', running: true },
    { id: 'c1d2e3f4a5b6', name: 'nginx', image: 'nginx:1.27-alpine', status: 'Up 12 days', ports: '0.0.0.0:80->80/tcp, 0.0.0.0:443->443/tcp', created: '2026-10-05 08:11:40', state: 'running', running: true },
    { id: 'd4e5f6a7b8c9', name: 'redis', image: 'redis:7.2', status: 'Up 12 days', ports: '127.0.0.1:6379->6379/tcp', created: '2026-10-05 08:11:41', state: 'running', running: true },
    { id: 'e7f8a9b0c1d2', name: 'migrate', image: 'ghcr.io/acme/api:2.14', status: 'Exited (0) 3 days ago', ports: '', created: '2026-10-14 10:01:58', state: 'exited', running: false },
  ]

  const security = {
    generated_at: now - 40, score: 78, passed: 7, scored: 9,
    checks: [
      { name: 'SSH root login', status: 'pass', message: 'PermitRootLogin is disabled', detail: null, remediation: null },
      { name: 'SSH password authentication', status: 'warn', message: 'PasswordAuthentication is enabled', detail: '/etc/ssh/sshd_config',
        remediation: { summary: 'Review the effective sshd settings', command: 'sudo sshd -T | grep -i passwordauthentication' } },
      { name: 'Firewall', status: 'pass', message: 'ufw is active with 3 rules', detail: null, remediation: null },
      { name: 'Pending security updates', status: 'fail', message: '4 security updates available', detail: 'openssl, libssl3, curl, libcurl4',
        remediation: { summary: 'Install pending updates', command: 'sudo apt update && sudo apt upgrade' } },
      { name: 'Automatic updates', status: 'pass', message: 'unattended-upgrades is enabled', detail: null, remediation: null },
      { name: 'Fail2ban', status: 'pass', message: 'fail2ban is running (sshd jail)', detail: null, remediation: null },
      { name: 'World-writable files in /etc', status: 'pass', message: 'None found', detail: null, remediation: null },
      { name: 'Listening services', status: 'pass', message: '3 public ports: 22, 80, 443', detail: null, remediation: null },
      { name: 'Time synchronisation', status: 'pass', message: 'systemd-timesyncd is synchronised', detail: null, remediation: null },
      { name: 'Kernel reboot required', status: 'unknown', message: 'Could not determine without sudo', detail: null, remediation: null },
    ],
  }

  const oid = i => ({ $oid: (0x66f1a2b3c4d5e6f7a8b9c0d1n + BigInt(i)).toString(16) })
  const names = ['Alice Martin', 'Bao Tran', 'Chen Wei', 'Dara Sok', 'Elena Rossi', 'Farah Khan', 'Gabriel Silva', 'Hana Sato', 'Ivan Petrov', 'Julia Novak', 'Kofi Mensah', 'Lina Haddad']
  const statuses = ['delivered', 'shipped', 'processing', 'delivered', 'delivered', 'cancelled']
  const orders = names.map((nm, i) => ({
    _id: oid(i), orderNo: `ORD-2026-${String(10421 + i).padStart(6, '0')}`, customer: nm,
    email: nm.split(' ')[0].toLowerCase() + '@example.com', total: { $numberDecimal: (39.9 + i * 17.35).toFixed(2) },
    items: (i % 4) + 1, status: statuses[i % statuses.length], createdAt: { $date: new Date(Date.UTC(2026, 9, 17 - i, 9 + i % 8, 12)).toISOString() },
  }))

  const rk = (text, kind, ttl) => ({ key: { text, binary: false, bytes: text.length, truncated: false }, key_b64: btoa(text), kind, ttl_ms: ttl, memory_bytes: null })
  const redisKeys = [
    rk('session:7f3a9c', 'string', 3540000), rk('session:b21e04', 'string', 1820000), rk('session:c9d118', 'string', 3290000),
    rk('user:1001', 'hash', -1), rk('user:1002', 'hash', -1), rk('user:1003', 'hash', -1),
    rk('cart:1001', 'list', 86400000), rk('leaderboard:weekly', 'zset', 604800000), rk('feature-flags', 'set', -1),
    rk('rate:203.0.113.7', 'string', 42000),
  ]
  const rv = text => ({ text, binary: false, bytes: text.length, truncated: false })
  const redisValues = {
    'user:1001': { kind: 'hash', entries: [['name', 'Alice Martin'], ['email', 'alice@example.com'], ['plan', 'pro'], ['created_at', '2025-03-14T09:21:00Z'], ['orders', '12'], ['locale', 'en-GB']]
      .map(([f, v]) => ({ field: rv(f), value: rv(v), score: null })), total: 6, ttl_ms: -1, encoding: 'listpack', cursor: '0', done: true },
  }

  const answers = {
    get_hosts: hosts,
    get_host_by_id: a => hosts.find(h => h.id === a.id),
    get_setting: a => ({ theme, font_size: '13' }[a?.key] ?? null),
    get_port_forwards: [
      { id: 1, host_id: 1, name: 'Postgres', kind: 'local', local_host: '127.0.0.1', local_port: 5433, remote_host: 'db.internal', remote_port: 5432 },
      { id: 2, host_id: 1, name: 'Grafana', kind: 'local', local_host: '127.0.0.1', local_port: 3001, remote_host: 'localhost', remote_port: 3000 },
      { id: 3, host_id: 1, name: 'SOCKS proxy', kind: 'dynamic', local_host: '127.0.0.1', local_port: 1080, remote_host: null, remote_port: null },
    ],
    get_port_forward_status: a => a.ruleId !== 3,
    ssh_connect: () => 'sess-' + nextId++,
    sftp_connect: () => 'sftp-' + nextId++,
    sftp_realpath: '/home/deploy',
    sftp_list: files,
    get_system_stats: stats,
    get_system_panel: panel,
    docker_ps: containers,
    run_security_audit: security,
    list_local_shells: [
      { id: 'unix:/usr/bin/zsh', label: 'zsh', program: '/usr/bin/zsh', args: ['-l'], default: true },
      { id: 'unix:/bin/bash', label: 'bash', program: '/bin/bash', args: ['-l'], default: false },
      { id: 'unix:/usr/bin/fish', label: 'fish', program: '/usr/bin/fish', args: ['-l'], default: false },
    ],
    mongodb_list_databases: ['admin', 'analytics', 'shop'],
    mongodb_list_collections: a => ({ shop: ['customers', 'inventory', 'orders', 'products'], analytics: ['events', 'sessions'], admin: ['system.users'] }[a.db] || []),
    mongodb_count: { count: 18342, estimated: false },
    mongodb_find: { documents: orders.map(o => JSON.stringify(o)), truncated: false, elapsed_ms: 23 },
    mongodb_collection_stats: JSON.stringify({ size: 9437184, nindexes: 4 }),
    redis_connect: { version: '7.2.4', mode: 'standalone', databases: [{ index: 0, keys: 18420, expires: 3120 }, { index: 1, keys: 214, expires: 0 }], database_count: 16, tunnelled: true, display_uri: 'redis://***@cache.internal:6379', supports_scan_type: true },
    redis_server_info: () => answers.redis_connect,
    redis_scan_keys: { keys: redisKeys, cursor: '0', done: true },
    redis_key_tree: { groups: [{ prefix: 'session', count: 3120, binary: false }, { prefix: 'user', count: 9800, binary: false }, { prefix: 'cart', count: 5210, binary: false }, { prefix: 'rate', count: 288, binary: false }, { prefix: 'leaderboard', count: 1, binary: false }], scanned: 18420, truncated: false },
    redis_key_value: a => redisValues[atob(a.keyB64)] || { kind: 'string', entries: [{ field: null, value: rv('{"uid":1001,"ip":"203.0.113.7","exp":1760712000}'), score: null }], total: 46, ttl_ms: 3540000, encoding: 'embstr', cursor: '0', done: true },
  }

  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
    transformCallback: (f) => { const id = nextId++; callbacks.set(id, f); return id },
    unregisterCallback: (id) => callbacks.delete(id),
    convertFileSrc: (p) => p,
    invoke: async (cmd, args) => {
      // Real IPC always answers after the current event has finished
      // dispatching; answering inside it lets click-outside handlers see a
      // half-updated DOM.
      await new Promise(r => setTimeout(r, 0))
      if (cmd === 'plugin:event|listen') {
        const id = nextId++
        if (!listeners.has(args.event)) listeners.set(args.event, [])
        listeners.get(args.event).push({ id, handler: args.handler })
        return id
      }
      if (cmd === 'open_data_channel') {
        const ch = args.channel
        let index = 0
        const send = (s) => callbacks.get(ch.id)?.({ index: index++, message: s })
        window.__demo.channels.push({ send })
        if (!q.has('nosession')) {
          let t = 300
          for (const part of session) setTimeout(() => send(part), (t += 25))
        } else {
          setTimeout(() => send(session.slice(0, 2).join('') + prompt), 300)
        }
        return null
      }
      const a = answers[cmd]
      if (typeof a === 'function') return a(args)
      if (a !== undefined) return a
      return null
    },
  }
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} }
})()
