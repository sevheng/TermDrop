// Terminal I/O benchmark (docs/terminal-io-plan.md, phase 0). Dev builds only:
// MainWindow imports this when started with `VITE_TERM_PERF=1 npm run tauri dev`,
// and Vite drops it from release builds.
//
// It drives real local terminal tabs through the real backend and webview:
// throughput of two output workloads, keystroke-to-echo latency, and a window
// of idle tabs. Each echo is also broken into stages, using backend
// timestamps from `dev_perf_marks` (src-tauri/src/dev_perf.rs) lined up with
// the webview's own on the shared wall clock. CPU is not visible from here; run `pidstat` alongside and cut
// its samples by the phase timestamps in the report. The report is written by
// the debug-only `dev_perf_report` command to <data dir>/termdrop/perf/.

import { invoke } from '../utils/invoke.js'
import { toast } from '../utils/toast.js'

const sleep = ms => new Promise(r => setTimeout(r, ms))
const epoch = () => Date.now() / 1000
// Wall-clock milliseconds at sub-millisecond resolution, comparable with the
// backend's SystemTime marks.
const wallMs = () => performance.timeOrigin + performance.now()

const WORKLOADS = {
  // ~14 MB of short lines: the common shape of build and log output.
  seq: 'seq 1 2000000',
  // ~8 MB of 76-column lines that do not compress into repeats.
  base64: 'head -c 6000000 /dev/urandom | base64',
}
const RUNS = 3
const ONLY = import.meta.env.VITE_TERM_PERF_ONLY // 'latency' skips the throughput and idle phases
const ECHO_SAMPLES = ONLY === 'latency' ? 400 : 200
const IDLE_SECONDS = 30
const IDLE_TABS = 5

/** Per-session counters, fed by useXtermInstance for every processed write. */
const probes = new Map()
window.__termPerf = {
  attach(sessionId, term, renderer = null) {
    const probe = {
      term, renderer, messages: 0, bytes: 0, lastProcessed: 0, echoStart: null, latencies: [],
      // Receive times of writes xterm has not processed yet, and how long each
      // waited: the backlog a heavy redraw builds up in the parser/renderer.
      inflight: [], writeDelays: [],
      decoder: new TextDecoder(), tail: '', waiting: null,
      // The echo being timed: wall-clock stamps filled in as it travels.
      echo: null,
    }
    probes.set(sessionId, probe)
    return {
      received() {
        probe.inflight.push(performance.now())
        if (probe.echo && probe.echo.received === undefined) probe.echo.received = wallMs()
      },
      processed(data) {
        if (probe.echo && probe.echo.processed === undefined) probe.echo.processed = wallMs()
        const now = performance.now()
        const got = probe.inflight.shift()
        if (got !== undefined) probe.writeDelays.push(now - got)
        probe.messages++
        probe.bytes += data.length
        probe.lastProcessed = now
        if (probe.waiting) {
          // Keep a short tail so a marker split across two writes still matches.
          const text = probe.tail + (typeof data === 'string' ? data : probe.decoder.decode(data, { stream: true }))
          if (text.includes(probe.waiting.marker)) {
            probe.waiting.resolve(now)
            probe.waiting = null
          }
          probe.tail = text.slice(-32)
        }
        if (probe.echoStart !== null) {
          probe.latencies.push(now - probe.echoStart)
          probe.echoStart = null
        }
      },
    }
  },
}

const percentile = (values, p) => {
  if (!values.length) return null
  const sorted = [...values].sort((a, b) => a - b)
  return sorted[Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length))]
}
const round = (n, digits = 1) => (n === null ? null : Number(n.toFixed(digits)))

/**
 * Run `command` and resolve with the time its last byte was processed, found
 * by a marker the shell prints afterwards. The marker is computed by the
 * shell, so the echoed command line itself cannot match it.
 *
 * Waiting for a quiet period instead was wrong: when the webview's main
 * thread stalls, the quiet timer fires before the queued output is
 * processed, and the run is declared over mid-stream.
 */
function runToMarker(sessionId, probe, command, timeoutMs = 300000) {
  const n = Math.floor(Math.random() * 1e6)
  const marker = `__PERF_DONE_${n + 1}__`
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { probe.waiting = null; reject(new Error('marker never arrived')) }, timeoutMs)
    probe.tail = ''
    probe.waiting = { marker, resolve: t => { clearTimeout(timer); resolve(t) } }
    send(sessionId, `${command}; echo __PERF_DONE_$((${n} + 1))__\r`)
  })
}

/** Wait until the session has produced nothing for `quietMs`. */
async function settle(probe, quietMs, timeoutMs = 120000) {
  const deadline = performance.now() + timeoutMs
  while (performance.now() < deadline) {
    if (probe.lastProcessed && performance.now() - probe.lastProcessed > quietMs) return
    await sleep(50)
  }
  throw new Error('terminal never went quiet')
}

const send = (sessionId, data) => invoke('ssh_write', { sessionId, data })

const WINDOWS = navigator.userAgent.includes('Windows')

async function openShell(store) {
  // On Windows, plain cmd: PowerShell's line editor redraws around each key.
  const shell = WINDOWS ? (await store.loadLocalShells()).find(s => s.id === 'cmd') ?? null : null
  const sessionId = await store.openLocalTerminal(shell)
  const deadline = performance.now() + 15000
  while (!probes.has(sessionId)) {
    if (performance.now() > deadline) throw new Error('terminal never attached')
    await sleep(50)
  }
  const probe = probes.get(sessionId)
  await settle(probe, 1500)
  // A plain shell, so the user's prompt and line-editor plugins do not
  // become part of what is measured.
  if (!WINDOWS) {
    await send(sessionId, "exec env PS1='$ ' /bin/sh\r")
    await sleep(300)
    await settle(probe, 1000)
  }
  return { sessionId, probe }
}

async function throughput(sessionId, probe, command) {
  const runs = []
  for (let i = 0; i < RUNS; i++) {
    await send(sessionId, 'clear\r')
    await settle(probe, 500)
    const before = { messages: probe.messages, bytes: probe.bytes }
    // How long the main thread went without running a 50 ms timer: the
    // freeze a user would feel while the output streams in.
    let lastTick = performance.now()
    let maxStall = 0
    const ticker = setInterval(() => {
      const now = performance.now()
      maxStall = Math.max(maxStall, now - lastTick)
      lastTick = now
    }, 50)
    const start = performance.now()
    const end = await runToMarker(sessionId, probe, command)
    clearInterval(ticker)
    const ms = end - start
    const bytes = probe.bytes - before.bytes
    const messages = probe.messages - before.messages
    runs.push({ ms: round(ms), bytes, messages, max_stall_ms: round(maxStall) })
  }
  const ms = percentile(runs.map(r => r.ms), 50)
  const median = runs.find(r => r.ms === ms)
  return {
    runs,
    median_ms: ms,
    mb_per_s: round(median.bytes / 1e6 / (ms / 1000), 2),
    bytes_per_message: Math.round(median.bytes / median.messages),
    max_stall_ms: round(Math.max(...runs.map(r => r.max_stall_ms))),
  }
}

// One keystroke's trip, in order. Backend stages come from dev_perf::mark.
const STAGES = [
  'input', // term.input(): TerminalTab sends single keys to ssh_write at once
  'cmd', // ssh_write starts running in the backend
  'pty_write', // the local loop has written the key to the PTY
  'read', // the reader thread has the shell's echo
  'loop_read', // the local loop has taken it from the reader thread
  'sent', // the loop has handed it to the data channel
  'received', // channel.onmessage in the webview
  'processed', // xterm's write callback: parsed into the buffer
]
// There is deliberately no "next frame" stage. Requesting an animation frame
// after each echo, or making an IPC call between samples, changes what is
// measured: while the page is busy, both IPC hops get faster and the median
// echo fell from 38 ms to 3 ms. Marks are therefore collected once, at the end.

/** Median, p95 and max of each stage-to-stage gap, plus the totals. */
function stageBreakdown(samples) {
  const stats = (values) => ({
    median_ms: round(percentile(values, 50), 2),
    p95_ms: round(percentile(values, 95), 2),
    max_ms: values.length ? round(Math.max(...values), 2) : null,
  })
  const complete = samples.filter(s => STAGES.every(k => typeof s[k] === 'number'))
  const gaps = {}
  for (let i = 1; i < STAGES.length; i++) {
    const values = complete.map(s => s[STAGES[i]] - s[STAGES[i - 1]])
    // Backend and webview stamps come from different processes; a negative
    // gap means they disagree, and the numbers around it should be doubted.
    gaps[`${STAGES[i - 1]}->${STAGES[i]}`] = { ...stats(values), negative: values.filter(v => v < 0).length }
  }
  return {
    n: complete.length,
    incomplete: samples.length - complete.length,
    gaps,
    total: stats(complete.map(s => s.processed - s.input)),
  }
}

async function echoLatency(probe) {
  probe.latencies = []
  const samples = []
  await invoke('dev_perf_marks', { record: true })
  for (let i = 0; i < ECHO_SAMPLES; i++) {
    if (i && i % 50 === 0) {
      probe.term.input('\x15', true) // ^U clears the line between batches
      await sleep(200)
    }
    const echo = { input: wallMs() }
    probe.echo = echo
    probe.echoStart = performance.now()
    probe.term.input('x', true)
    const deadline = performance.now() + 1000
    while (echo.processed === undefined && performance.now() < deadline) await sleep(1)
    probe.echoStart = null
    probe.echo = null
    samples.push(echo)
    await sleep(30)
  }
  // Each sample owns the marks from its keystroke up to the next one. Its
  // echo comes first in that window; a ^U between batches comes after it.
  const marks = await invoke('dev_perf_marks', { record: false })
  samples.forEach((echo, i) => {
    const end = samples[i + 1]?.input ?? Infinity
    for (const [stage, ms] of marks) {
      if (ms >= echo.input && ms < end && echo[stage] === undefined) echo[stage] = ms
    }
  })
  probe.term.input('\x15', true)
  const l = probe.latencies
  return {
    n: l.length,
    median_ms: round(percentile(l, 50), 2),
    p95_ms: round(percentile(l, 95), 2),
    max_ms: round(Math.max(...l), 2),
    stages: stageBreakdown(samples),
  }
}

// A real full-screen app instead of a synthetic workload: copane (`ce`) on a
// large file in a WSL tab. Only editor keys are sent -- arrows, PageUp/Down,
// Ctrl+Home, Ctrl+E (focus editor) and Ctrl+Q -- never text, so nothing can
// reach the AI pane beside it.
const APP_DISTRO = import.meta.env.VITE_TERM_PERF_DISTRO || 'Ubuntu-20.04'
const APP_COMMAND = import.meta.env.VITE_TERM_PERF_APP
  || 'cd ~/Development-Personal/TermDrop/ssh-client && ce src-tauri/src/main.rs'
const KEY = { down: '\x1b[B', pageDown: '\x1b[6~', ctrlHome: '\x1b[1;5H', ctrlE: '\x05', ctrlQ: '\x11' }

const stats = (values) => ({
  n: values.length,
  median_ms: round(percentile(values, 50), 2),
  p95_ms: round(percentile(values, 95), 2),
  max_ms: values.length ? round(Math.max(...values), 2) : null,
})

/** Watch the main thread for stalls while `fn` runs, as `throughput` does. */
async function withStallTicker(fn) {
  let lastTick = performance.now()
  let maxStall = 0
  const ticker = setInterval(() => {
    const now = performance.now()
    maxStall = Math.max(maxStall, now - lastTick)
    lastTick = now
  }, 50)
  try {
    const result = await fn()
    return { ...result, max_stall_ms: round(maxStall) }
  } finally {
    clearInterval(ticker)
  }
}

/** Key → first processed output, like echoLatency but for an app's redraw. */
async function keyLatency(probe, key, samples) {
  probe.latencies = []
  const echoes = []
  await invoke('dev_perf_marks', { record: true })
  for (let i = 0; i < samples; i++) {
    const echo = { input: wallMs() }
    probe.echo = echo
    probe.echoStart = performance.now()
    probe.term.input(key, true)
    const deadline = performance.now() + 1000
    while (probe.echoStart !== null && performance.now() < deadline) await sleep(1)
    probe.echoStart = null
    probe.echo = null
    echoes.push(echo)
    await sleep(60)
  }
  const marks = await invoke('dev_perf_marks', { record: false })
  echoes.forEach((echo, i) => {
    const end = echoes[i + 1]?.input ?? Infinity
    for (const [stage, ms] of marks) {
      if (ms >= echo.input && ms < end && echo[stage] === undefined) echo[stage] = ms
    }
  })
  return { ...stats(probe.latencies), stages: stageBreakdown(echoes) }
}

/**
 * Hold-the-key scrolling: `count` PageDowns at key-repeat speed, then how long
 * the screen keeps catching up after the last one.
 */
async function scrollBurst(probe, count, intervalMs) {
  await sleep(300)
  probe.writeDelays = []
  const before = { messages: probe.messages, bytes: probe.bytes }
  return withStallTicker(async () => {
    const start = performance.now()
    for (let i = 0; i < count; i++) {
      probe.term.input(KEY.pageDown, true)
      await sleep(intervalMs)
    }
    const lastKey = performance.now()
    await settle(probe, 250, 30000).catch(() => {})
    const caughtUp = probe.lastProcessed
    probe.term.input(KEY.ctrlHome, true)
    await settle(probe, 250, 30000).catch(() => {})
    return {
      keys: count,
      typing_ms: round(lastKey - start),
      lag_after_last_key_ms: round(Math.max(0, caughtUp - lastKey)),
      bytes: probe.bytes - before.bytes,
      messages: probe.messages - before.messages,
      write_backlog: stats(probe.writeDelays),
    }
  })
}

async function appBenchmark(store, report, phase) {
  const shells = await store.loadLocalShells()
  const shell = shells.find(s => s.id === `wsl:${APP_DISTRO}`) ?? null
  if (!shell) throw new Error(`no WSL shell wsl:${APP_DISTRO}; have ${shells.map(s => s.id).join(', ')}`)
  const sessionId = await store.openLocalTerminal(shell)
  const deadline = performance.now() + 15000
  while (!probes.has(sessionId)) {
    if (performance.now() > deadline) throw new Error('terminal never attached')
    await sleep(50)
  }
  const probe = probes.get(sessionId)
  await settle(probe, 1500, 30000)
  report.terminal = { cols: probe.term.cols, rows: probe.term.rows, gpu_at_start: probe.renderer?.gpu() ?? null }

  await send(sessionId, `${APP_COMMAND}\r`)
  // The AI pane keeps starting for a while; give it time rather than waiting
  // for a quiet screen that a spinner may never allow.
  await sleep(15000)
  probe.term.input(KEY.ctrlE, true)
  await sleep(1000)

  await phase('app-idle', async () => {
    const before = { messages: probe.messages, bytes: probe.bytes }
    await sleep(10000)
    return { seconds: 10, bytes: probe.bytes - before.bytes, messages: probe.messages - before.messages }
  })

  for (const mode of ['as-is', 'dom']) {
    if (mode === 'dom') {
      if (!probe.renderer?.gpu()) break // already DOM: nothing to compare
      probe.renderer.detachGpuRenderer()
      await sleep(500)
    }
    const gpu = probe.renderer?.gpu() ?? null
    await phase(`app-arrow-latency-${mode}`, async () => ({ gpu, ...(await keyLatency(probe, KEY.down, 100)) }))
    probe.term.input(KEY.ctrlHome, true)
    for (let run = 0; run < 3; run++) {
      await phase(`app-scroll-${mode}-${run}`, async () => ({ gpu, ...(await scrollBurst(probe, 40, 33)) }))
    }
  }
  probe.renderer?.attachGpuRenderer()

  probe.term.input(KEY.ctrlQ, true)
  await sleep(3000)
  return sessionId
}

export async function runTermPerf(store) {
  const name = import.meta.env.VITE_TERM_PERF_NAME || `run-${new Date().toISOString().replace(/[:.]/g, '-')}`
  const report = { name, user_agent: navigator.userAgent, started: new Date().toISOString(), phases: [] }
  const phase = async (label, fn) => {
    const start = epoch()
    const result = await fn()
    report.phases.push({ name: label, start_epoch: round(start, 3), end_epoch: round(epoch(), 3), ...result })
  }
  const opened = []
  toast('Terminal benchmark running — leave the window alone', 'info')
  try {
    await sleep(2000)
    if (ONLY === 'app') {
      opened.push(await appBenchmark(store, report, phase))
    } else {
      if (!ONLY) await phase('idle-no-tabs', async () => { await sleep(10000); return {} })

      const main = await openShell(store)
      opened.push(main.sessionId)
      if (!ONLY) {
        for (const [label, command] of Object.entries(WORKLOADS)) {
          await phase(`throughput-${label}`, () => throughput(main.sessionId, main.probe, command))
        }
      }
      await phase('echo-latency', () => echoLatency(main.probe))

      if (!ONLY) {
        for (let i = 1; i < IDLE_TABS; i++) opened.push((await openShell(store)).sessionId)
        store.setActiveTab(main.sessionId)
        await sleep(2000)
        await phase(`idle-${IDLE_TABS}-tabs`, async () => { await sleep(IDLE_SECONDS * 1000); return { tabs: opened.length } })
      }
    }

    report.finished = new Date().toISOString()
    const path = await invoke('dev_perf_report', { name, report: JSON.stringify(report, null, 2) })
    console.log('[termPerf]', report)
    toast(`Terminal benchmark written to ${path}`, 'success')
  } catch (err) {
    console.error('[termPerf] failed', err)
    toast('Terminal benchmark failed: ' + err, 'error')
  } finally {
    for (const id of opened) await store.disconnect(id).catch(() => {})
  }
}
