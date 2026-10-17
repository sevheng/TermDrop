// Terminal I/O benchmark (docs/terminal-io-plan.md, phase 0). Dev builds only:
// MainWindow imports this when started with `VITE_TERM_PERF=1 npm run tauri dev`,
// and Vite drops it from release builds.
//
// It drives real local terminal tabs through the real backend and webview:
// throughput of two output workloads, keystroke-to-echo latency, and a window
// of idle tabs. CPU is not visible from here; run `pidstat` alongside and cut
// its samples by the phase timestamps in the report. The report is written by
// the debug-only `dev_perf_report` command to <data dir>/termdrop/perf/.

import { invoke } from '../utils/invoke.js'
import { toast } from '../utils/toast.js'

const sleep = ms => new Promise(r => setTimeout(r, ms))
const epoch = () => Date.now() / 1000

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
  attach(sessionId, term) {
    const probe = {
      term, messages: 0, bytes: 0, lastProcessed: 0, echoStart: null, latencies: [],
      decoder: new TextDecoder(), tail: '', waiting: null,
    }
    probes.set(sessionId, probe)
    return {
      processed(data) {
        const now = performance.now()
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

async function openShell(store) {
  const sessionId = await store.openLocalTerminal()
  const deadline = performance.now() + 15000
  while (!probes.has(sessionId)) {
    if (performance.now() > deadline) throw new Error('terminal never attached')
    await sleep(50)
  }
  const probe = probes.get(sessionId)
  await settle(probe, 1500)
  // A plain shell, so the user's prompt and line-editor plugins do not
  // become part of what is measured.
  await send(sessionId, "exec env PS1='$ ' /bin/sh\r")
  await sleep(300)
  await settle(probe, 1000)
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

async function echoLatency(probe) {
  probe.latencies = []
  for (let i = 0; i < ECHO_SAMPLES; i++) {
    if (i && i % 50 === 0) {
      probe.term.input('\x15', true) // ^U clears the line between batches
      await sleep(200)
    }
    probe.echoStart = performance.now()
    probe.term.input('x', true)
    const deadline = performance.now() + 1000
    while (probe.echoStart !== null && performance.now() < deadline) await sleep(1)
    probe.echoStart = null
    await sleep(30)
  }
  probe.term.input('\x15', true)
  const l = probe.latencies
  return { n: l.length, median_ms: round(percentile(l, 50), 2), p95_ms: round(percentile(l, 95), 2), max_ms: round(Math.max(...l), 2) }
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
