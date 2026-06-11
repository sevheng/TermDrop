<template>
  <div class="h-full flex flex-col bg-[#1e1e1e]">
    <!-- Toolbar -->
    <div class="flex items-center justify-between px-2 py-1 border-b border-[#3c3c3c]">
      <div class="flex items-center gap-2">
        <span class="text-[10px] text-[#6e6e6e]">{{ report?.checks?.length || 0 }} checks</span>
        <span v-if="timeAgo" class="text-[10px] text-[#858585]">· updated {{ timeAgo }}</span>
      </div>
      <button
        @click="runAudit(true)"
        :disabled="loading"
        class="text-xs text-[#858585] hover:text-[#cccccc] disabled:text-[#3c3c3c] flex items-center gap-1"
      >
        <RefreshCw :size="12" :class="loading && 'animate-spin'" />
        Re-run
      </button>
    </div>

    <!-- Content -->
    <div class="flex-1 overflow-y-auto">
      <!-- Loading (background audit running) -->
      <div v-if="loading" class="flex flex-col items-center justify-center py-12">
        <Loader2 :size="20" class="animate-spin text-[#858585] mb-2" />
        <span class="text-xs text-[#858585]">Running security audit...</span>
        <span class="text-[10px] text-[#6e6e6e] mt-1">Switch tabs freely — results will appear here</span>
      </div>

      <!-- Error -->
      <div v-else-if="error" class="flex flex-col items-center justify-center py-12 text-[#6e6e6e]">
        <ShieldAlert :size="24" class="mb-2 text-[#f44336] opacity-50" />
        <p class="text-xs text-[#f44336]">Audit failed</p>
        <p class="text-[10px] mt-1">{{ error }}</p>
        <button
          @click="runAudit(true)"
          class="mt-3 px-3 py-1 bg-[#0e639c] hover:bg-[#1177bb] text-white text-xs rounded"
        >
          Retry
        </button>
      </div>

      <!-- Empty -->
      <div v-else-if="!report" class="flex flex-col items-center justify-center py-12 text-[#6e6e6e]">
        <Shield :size="24" class="mb-2 opacity-50" />
        <p class="text-xs">No audit data</p>
        <p class="text-[10px] mt-1">Connect to a host to run audit</p>
        <button
          @click="runAudit(true)"
          class="mt-3 px-3 py-1 bg-[#0e639c] hover:bg-[#1177bb] text-white text-xs rounded"
        >
          Run Audit
        </button>
      </div>

      <!-- Report -->
      <div v-else>
        <!-- Score header -->
        <div class="flex items-center justify-center py-4 border-b border-[#3c3c3c]">
          <div class="text-center">
            <div
              class="w-14 h-14 rounded-full flex items-center justify-center text-lg font-bold mx-auto mb-1"
              :class="scoreClass"
            >
              {{ report.score }}
            </div>
            <span class="text-[10px] text-[#858585] uppercase tracking-wide">{{ scoreLabel }}</span>
          </div>
        </div>

        <!-- Checks list -->
        <div class="divide-y divide-[#3c3c3c]/50">
          <div
            v-for="check in report.checks"
            :key="check.name"
            class="px-2 py-1.5 flex items-start gap-2"
          >
            <component
              :is="iconFor(check.status)"
              :size="14"
              class="shrink-0 mt-0.5"
              :class="colorFor(check.status)"
            />
            <div class="flex-1 min-w-0">
              <div class="flex items-center gap-1.5">
                <span class="text-[11px] text-[#cccccc]">{{ check.name }}</span>
                <span
                  class="text-[9px] px-1 py-0 rounded font-medium uppercase"
                  :class="badgeClassFor(check.status)"
                >
                  {{ check.status }}
                </span>
              </div>
              <p class="text-[10px] text-[#858585] mt-0.5">{{ check.message }}</p>
              <p v-if="check.detail" class="text-[10px] text-[#6e6e6e] mt-0.5 font-mono truncate">{{ check.detail }}</p>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, onUnmounted, watch, computed } from 'vue'
import { useConnectionStore } from '../stores/connection.js'
import { RefreshCw, Loader2, Shield, ShieldCheck, ShieldAlert, AlertTriangle, XCircle } from 'lucide-vue-next'

const props = defineProps({
  hostId: {
    type: Number,
    required: true,
  },
})

const store = useConnectionStore()

const report = ref(null)
const loading = ref(false)
const error = ref(null)
const lastUpdated = ref(null)

const timeAgo = computed(() => {
  if (!lastUpdated.value) return ''
  const seconds = Math.floor((Date.now() - lastUpdated.value) / 1000)
  if (seconds < 5) return 'just now'
  if (seconds < 60) return `${seconds}s ago`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.floor(minutes / 60)
  return `${hours}h ago`
})

const scoreLabel = computed(() => {
  if (!report.value) return ''
  const s = report.value.score
  if (s >= 80) return 'Good'
  if (s >= 50) return 'Fair'
  return 'Poor'
})

const scoreClass = computed(() => {
  if (!report.value) return ''
  const s = report.value.score
  if (s >= 80) return 'bg-[#89d185]/20 text-[#89d185]'
  if (s >= 50) return 'bg-[#cca700]/20 text-[#cca700]'
  return 'bg-[#f44336]/20 text-[#f44336]'
})

function iconFor(status) {
  switch (status) {
    case 'pass': return ShieldCheck
    case 'warn': return AlertTriangle
    case 'fail': return XCircle
    default: return Shield
  }
}

function colorFor(status) {
  switch (status) {
    case 'pass': return 'text-[#89d185]'
    case 'warn': return 'text-[#cca700]'
    case 'fail': return 'text-[#f44336]'
    default: return 'text-[#858585]'
  }
}

function badgeClassFor(status) {
  switch (status) {
    case 'pass': return 'bg-[#89d185]/20 text-[#89d185]'
    case 'warn': return 'bg-[#cca700]/20 text-[#cca700]'
    case 'fail': return 'bg-[#f44336]/20 text-[#f44336]'
    default: return 'bg-[#3c3c3c] text-[#858585]'
  }
}

function readFromCache() {
  if (!props.hostId) {
    report.value = null
    loading.value = false
    error.value = null
    lastUpdated.value = null
    return
  }
  const cached = store.getSecurityReport(props.hostId)
  if (cached) {
    report.value = cached.report
    loading.value = cached.loading
    error.value = cached.error
    if (cached.report && !lastUpdated.value) {
      lastUpdated.value = Date.now()
    }
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

onMounted(readFromCache)

watch(() => props.hostId, readFromCache)

// Reactive: re-read from cache whenever the store version changes
watch(() => store.securityReportVersion, readFromCache)

</script>
