<template>
  <div class="h-full flex flex-col bg-canvas">
    <!-- Toolbar -->
    <div class="flex items-center justify-between px-2 py-1 border-b border-line">
      <div class="flex items-center gap-2">
        <span class="text-[10px] text-ink-3">{{ report?.checks?.length || 0 }} checks</span>
        <span v-if="timeAgo" class="text-[10px] text-ink-2">· updated {{ timeAgo }}</span>
      </div>
      <button
        @click="runAudit(true)"
        :disabled="loading"
        class="text-xs text-ink-2 hover:text-ink disabled:text-ink-3 disabled:opacity-60 flex items-center gap-1"
      >
        <RefreshCw :size="12" :class="loading && 'animate-spin'" />
        Re-run
      </button>
    </div>

    <!-- Content -->
    <div class="flex-1 overflow-y-auto">
      <!-- Loading (background audit running) -->
      <div v-if="loading" class="flex flex-col items-center justify-center py-12">
        <Loader2 :size="20" class="animate-spin text-ink-2 mb-2" />
        <span class="text-xs text-ink-2">Running security audit...</span>
        <span class="text-[10px] text-ink-3 mt-1">Switch tabs freely — results will appear here</span>
      </div>

      <!-- Error -->
      <div v-else-if="error" class="flex flex-col items-center justify-center py-12 text-ink-3">
        <ShieldAlert :size="24" class="mb-2 text-bad opacity-50" />
        <p class="text-xs text-bad">Audit failed</p>
        <p class="text-[10px] mt-1">{{ error }}</p>
        <button
          @click="runAudit(true)"
          class="mt-3 px-3 py-1 bg-accent-solid hover:bg-accent-solid-hover text-white text-xs rounded"
        >
          Retry
        </button>
      </div>

      <!-- Empty -->
      <div v-else-if="!report" class="flex flex-col items-center justify-center py-12 text-ink-3">
        <Shield :size="24" class="mb-2 opacity-50" />
        <p class="text-xs">No audit has run for this host</p>
        <p class="text-[10px] mt-1">The audit runs privileged probes on the server</p>
        <button
          @click="runAudit(true)"
          class="mt-3 px-3 py-1 bg-accent-solid hover:bg-accent-solid-hover text-white text-xs rounded"
        >
          Run Audit
        </button>
      </div>

      <!-- Report -->
      <div v-else>
        <!-- Score header -->
        <div class="flex items-center justify-center py-4 border-b border-line">
          <div class="text-center">
            <div
              class="w-14 h-14 rounded-full flex items-center justify-center text-lg font-bold mx-auto mb-1"
              :class="scoreClass"
            >
              {{ hasScore ? report.score : '—' }}
            </div>
            <span class="text-[10px] text-ink-2 uppercase tracking-wide">{{ scoreLabel }}</span>
            <p class="text-[10px] text-ink-2 mt-1">
              {{ hasScore ? `${report.passed} of ${report.scored} checks passed` : 'Nothing could be determined' }}
            </p>
            <p v-if="breakdown" class="text-[10px] text-ink-3 mt-0.5">{{ breakdown }}</p>
          </div>
        </div>

        <!-- Checks list -->
        <div class="divide-y divide-line/50">
          <div
            v-for="check in report.checks"
            :key="check.name"
            class="px-2 py-1.5 flex items-start gap-2"
          >
            <component
              :is="statusMeta(check.status).icon"
              :size="14"
              class="shrink-0 mt-0.5"
              :class="statusMeta(check.status).color"
            />
            <div class="flex-1 min-w-0">
              <div class="flex items-center gap-1.5">
                <span class="text-[11px] text-ink">{{ check.name }}</span>
                <span
                  class="text-[9px] px-1 py-0 rounded font-medium uppercase"
                  :class="statusMeta(check.status).badge"
                >
                  {{ check.status }}
                </span>
              </div>
              <p class="text-[10px] text-ink-2 mt-0.5">{{ check.message }}</p>
              <p
                v-if="check.detail"
                class="text-[10px] text-ink-3 mt-0.5 font-mono whitespace-pre-wrap break-words"
              >{{ check.detail }}</p>

              <!-- What to do about it. Collapsed by default so the list stays
                   scannable; a finding you are acting on is usually one. -->
              <template v-if="check.remediation">
                <button
                  @click="toggle(check.name)"
                  class="text-[10px] text-accent-soft hover:text-accent-soft-hover mt-1 flex items-center gap-0.5"
                >
                  <ChevronRight
                    :size="10"
                    class="transition-transform"
                    :class="isOpen(check.name) && 'rotate-90'"
                  />
                  What to do
                </button>
                <div v-if="isOpen(check.name)" class="mt-1 mb-0.5">
                  <p class="text-[10px] text-ink-2 leading-relaxed">
                    {{ check.remediation.summary }}
                  </p>
                  <template v-if="check.remediation.command">
                    <pre
                      class="mt-1 px-1.5 py-1 bg-surface border border-line rounded text-[10px] text-ink font-mono whitespace-pre-wrap break-all"
                    >{{ check.remediation.command }}</pre>
                    <div class="flex items-center gap-2 mt-1">
                      <button
                        @click="copyCommand(check.remediation.command)"
                        class="text-[10px] text-ink-2 hover:text-ink flex items-center gap-1"
                      >
                        <Copy :size="10" />
                        Copy
                      </button>
                      <button
                        v-if="canSend"
                        @click="sendCommand(check.remediation.command)"
                        class="text-[10px] text-ink-2 hover:text-ink flex items-center gap-1"
                      >
                        <TerminalSquare :size="10" />
                        Send to terminal
                      </button>
                    </div>
                    <p v-if="canSend" class="text-[9px] text-ink-3 mt-0.5">
                      Typed at the prompt without running. Press Enter yourself.
                    </p>
                  </template>
                </div>
              </template>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, reactive, onMounted, onUnmounted, onActivated, watch, computed } from 'vue'
import { useConnectionStore } from '../stores/connection.js'
import { RefreshCw, Loader2, Shield, ShieldCheck, ShieldAlert, AlertTriangle, XCircle, ChevronRight, Copy, TerminalSquare } from 'lucide-vue-next'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { isInsertableCommand, canReceiveCommand } from '../utils/terminalInsert.js'

const props = defineProps({
  hostId: {
    type: Number,
    required: true,
  },
})

const emit = defineEmits(['sendToTerminal'])

const store = useConnectionStore()

/** MongoDB tabs have no shell, and a disconnected tab has nothing to write to. */
const canSend = computed(() => canReceiveCommand(store.activeTab))

async function copyCommand(command) {
  try {
    await writeText(command)
  } catch {
    // Clipboard access can be refused; the command is on screen either way.
  }
}

/**
 * Hands the command to MainWindow, which knows the active session. It is typed
 * at the prompt and deliberately not submitted, so the user reads it and
 * presses Enter. The guard is a second check on top of the backend's, which
 * only ever emits its own literals.
 */
function sendCommand(command) {
  if (!canSend.value || !isInsertableCommand(command)) return
  emit('sendToTerminal', command)
}

const report = ref(null)
const loading = ref(false)
const error = ref(null)
const lastUpdated = ref(null)

// Re-read on a timer so the label ages instead of freezing at "just now".
const now = ref(Date.now())
let clock = null

const timeAgo = computed(() => {
  if (!lastUpdated.value) return ''
  const seconds = Math.max(0, Math.floor((now.value - lastUpdated.value) / 1000))
  if (seconds < 5) return 'just now'
  if (seconds < 60) return `${seconds}s ago`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.floor(minutes / 60)
  return `${hours}h ago`
})

/** False when every check came back undetermined, so there is nothing to grade. */
const hasScore = computed(() => (report.value?.scored ?? 0) > 0)

const scoreLabel = computed(() => {
  if (!report.value) return ''
  if (!hasScore.value) return 'Unknown'
  const s = report.value.score
  if (s >= 80) return 'Good'
  if (s >= 50) return 'Fair'
  return 'Poor'
})

const scoreClass = computed(() => {
  if (!report.value) return ''
  if (!hasScore.value) return 'bg-input text-ink-2'
  const s = report.value.score
  if (s >= 80) return 'bg-good/20 text-good'
  if (s >= 50) return 'bg-warn/20 text-warn'
  return 'bg-bad/20 text-bad'
})

/** "2 failed · 1 warning · 3 undetermined", omitting whichever counts are zero. */
const breakdown = computed(() => {
  const checks = report.value?.checks
  if (!checks?.length) return ''
  const count = (status) => checks.filter((c) => c.status === status).length
  const parts = []
  const failed = count('fail')
  const warned = count('warn')
  const unknown = count('unknown')
  if (failed) parts.push(`${failed} failed`)
  if (warned) parts.push(`${warned} warning${warned === 1 ? '' : 's'}`)
  if (unknown) parts.push(`${unknown} undetermined`)
  return parts.join(' · ')
})

const STATUS_META = {
  pass: { icon: ShieldCheck, color: 'text-good', badge: 'bg-good/20 text-good' },
  warn: { icon: AlertTriangle, color: 'text-warn', badge: 'bg-warn/20 text-warn' },
  fail: { icon: XCircle, color: 'text-bad', badge: 'bg-bad/20 text-bad' },
  // The backend reports this when a probe lacked the privileges to answer.
  unknown: { icon: Shield, color: 'text-ink-2', badge: 'bg-input text-ink-2' },
}
const UNKNOWN_STATUS_META = STATUS_META.unknown

function statusMeta(status) {
  return STATUS_META[status] || UNKNOWN_STATUS_META
}

/** Which findings have their guidance expanded, keyed by check name. */
const expanded = reactive(new Set())

function isOpen(name) {
  return expanded.has(name)
}

function toggle(name) {
  if (expanded.has(name)) expanded.delete(name)
  else expanded.add(name)
}

function readFromCache() {
  if (!props.hostId) {
    report.value = null
    loading.value = false
    error.value = null
    lastUpdated.value = null
    return
  }
  // A new report can renumber or drop checks, so stale expansions are cleared.
  expanded.clear()
  const cached = store.getSecurityReport(props.hostId)
  if (cached) {
    report.value = cached.report
    loading.value = cached.loading
    error.value = cached.error
    // From the audit's own timestamp, not from when this panel read it. Both
    // the backend and the store cache the report, so the two differ.
    lastUpdated.value = cached.report?.generated_at
      ? cached.report.generated_at * 1000
      : null
  } else {
    report.value = null
    loading.value = false
    error.value = null
    lastUpdated.value = null
  }
}

function runAudit(force = false) {
  if (!props.hostId) return
  store.runSecurityAudit(props.hostId, force)
  readFromCache()
}

/**
 * The audit runs privileged probes on the server, so it starts when the panel
 * is actually opened rather than on every connect. Once per host is enough,
 * because the store keeps the report across panel switches.
 */
let attempted = false

function maybeRun() {
  if (attempted || !props.hostId) return
  // Anything already present — a report, a run in flight, or an error — is
  // left alone. Not retrying an error matters: the panel is inside KeepAlive,
  // so a host that always fails would otherwise re-audit on every tab switch.
  if (store.getSecurityReport(props.hostId)) return
  attempted = true
  runAudit(false)
}

onMounted(() => {
  readFromCache()
  maybeRun()
  clock = setInterval(() => { now.value = Date.now() }, 15000)
})

onUnmounted(() => {
  clearInterval(clock)
  clock = null
})

// Fires on every panel-tab switch, and after KeepAlive evicts and remounts.
// KeepAlive can leave this mounted but hidden, so the age label is refreshed
// here too: it only needs to be right while the panel is on screen.
onActivated(() => {
  now.value = Date.now()
  maybeRun()
})

watch(() => props.hostId, () => {
  attempted = false
  readFromCache()
  maybeRun()
})

// securityReports is a reactive Map, so this fires whenever the entry for this
// host is replaced by setSecurityLoading/Report/Error, or removed on disconnect.
watch(() => store.getSecurityReport(props.hostId), (entry) => {
  // A disconnect drops the report; allow a fresh audit next time this panel
  // is opened for the host.
  if (!entry) attempted = false
  readFromCache()
})

</script>
