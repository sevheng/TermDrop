<template>
  <div class="flex flex-col h-full bg-canvas">
    <!-- Toolbar -->
    <PanelHeader
      :title="host?.name || 'Redis'"
      :icon="Layers"
      icon-class="text-redis"
      :subtitle="displayUri"
      subtitle-mono
      :meta="serverInfo ? `Redis ${serverInfo.version}` : ''"
    >
      <template #badges>
        <span
          v-if="serverInfo?.tunnelled"
          class="text-2xs px-1.5 py-0.5 rounded bg-active text-syn-cyan shrink-0"
          :title="`Tunnelled through ${tunnelHostName}`"
        >
          via {{ tunnelHostName }}
        </span>
        <span
          v-if="isCluster"
          class="text-2xs px-1.5 py-0.5 rounded bg-warn-bg text-warn-soft shrink-0"
          title="SCAN sees only this node, so the key list is one node's keyspace and backup is disabled"
        >
          cluster
        </span>
      </template>

      <template #actions>
        <IconButton :icon="RefreshCw" label="Refresh" :disabled="busy" @click="refreshAll" />
        <button
          :disabled="busy || !serverInfo || isCluster"
          :title="isCluster ? 'A cluster backup would cover only this node' : ''"
          @click="backup.backup(pattern)"
          class="text-xs px-2 py-1 rounded text-ink hover:bg-raised disabled:opacity-40"
        >
          Back up
        </button>
        <button
          :disabled="busy || !serverInfo"
          @click="backup.chooseRestoreFile()"
          class="text-xs px-2 py-1 rounded text-ink hover:bg-raised disabled:opacity-40"
        >
          Restore
        </button>
      </template>
    </PanelHeader>

    <!-- Progress -->
    <div
      v-if="busy"
      class="flex items-center gap-3 px-3 py-2 border-b border-line shrink-0"
    >
      <span class="text-xs text-ink shrink-0">{{ currentAction }}</span>
      <div class="flex-1 h-1.5 bg-input rounded overflow-hidden">
        <div
          class="h-full bg-accent-solid transition-all"
          :style="{ width: `${progress?.percent ?? 0}%` }"
        ></div>
      </div>
      <span class="text-2xs text-ink-2 shrink-0 w-48 truncate text-right tabular-nums">
        {{ progress?.detail || '…' }}
      </span>
      <button
        @click="backup.cancel()"
        class="text-xs px-2 py-0.5 rounded text-bad hover:bg-input shrink-0"
      >
        Cancel
      </button>
    </div>

    <!-- Connection error -->
    <div
      v-if="connectError"
      class="flex items-start gap-2 px-3 py-2 bg-bad-bg border-b border-bad-line shrink-0"
    >
      <AlertCircle :size="14" class="text-bad shrink-0 mt-0.5" />
      <p class="text-xs text-bad flex-1">{{ connectError }}</p>
      <button
        @click="connect"
        class="text-xs px-2 py-0.5 rounded text-ink hover:bg-input shrink-0"
      >
        Reconnect
      </button>
    </div>

    <div v-if="connecting" class="flex items-center justify-center flex-1">
      <Loader2 :size="20" class="animate-spin text-ink-2" />
    </div>

    <div v-else-if="serverInfo" class="flex-1 flex overflow-hidden min-h-0">
      <div class="w-64 shrink-0 border-r border-line flex flex-col">
        <RedisKeyTree
          :databases="databases"
          :activeDb="activeDb"
          :groups="tree.groups"
          :activeGroup="activeGroup"
          :scanned="tree.scanned"
          :truncated="tree.truncated"
          :treeLoading="treeLoading"
          @select-db="selectDb"
          @select-group="selectGroup"
        />
      </div>

      <div class="flex-1 min-w-0 flex">
        <div class="flex-1 min-w-0">
          <RedisKeyList
            :keys="keyPage"
            :pattern="pattern"
            :typeFilter="typeFilter"
            :showMemory="withMemory"
            :loading="keysLoading"
            :error="keysError"
            :done="keysDone"
            :hasNext="hasNext"
            :canGoBack="canGoBack"
            :pageNumber="pageNumber"
            :summary="keysSummary"
            :activeKeyB64="activeKey?.key_b64"
            @update:pattern="pattern = $event"
            @update:typeFilter="typeFilter = $event"
            @update:showMemory="withMemory = $event"
            @search="onSearch"
            @open-key="openKey"
            @back="keys.back()"
            @forward="keys.forward()"
          />
        </div>
        <div class="w-96 shrink-0">
          <RedisValueView
            :keyB64="activeKey?.key_b64"
            :keyLabel="activeKeyLabel"
            :page="valuePage"
            :loading="valueLoading"
            :error="valueError"
            @more="loadMoreValue"
          />
        </div>
      </div>
    </div>

    <RedisRestoreDialog
      :state="restoreConfirm"
      :connectionName="host?.name || ''"
      :serverInfo="serverInfo"
      @confirm="backup.confirmRestore()"
      @cancel="backup.cancelRestore()"
      @update:replace="restoreConfirm.replace = $event"
      @update:flushFirst="restoreConfirm.flushFirst = $event"
      @update:targetDb="restoreConfirm.targetDb = $event"
    />
  </div>
</template>

<script setup>
/**
 * A Redis connection: browse it, back it up, restore it.
 *
 * The panel owns the connection rather than the store, so a tab appears
 * immediately and shows its own spinner and error banner. That matters more
 * here than for MongoDB, because a tunnelled connection also has to bring up
 * an SSH session and its failures are the ones worth reading.
 */
import { ref, computed, onMounted, watch } from 'vue'
import { Layers, Loader2, AlertCircle, RefreshCw } from 'lucide-vue-next'
import { invoke } from '../utils/invoke.js'
import { toast } from '../utils/toast.js'
import { redisDisplayUri } from '../utils/redisUri.js'
import { prefixPattern } from '../utils/redisKeys.js'
import { MAX_ELEMENTS_PER_PAGE } from '../utils/redisLimits.js'
import { useConnectionStore } from '../stores/connection.js'
import { useListenerGroup } from '../composables/useListenerGroup.js'
import { useRedisConnection } from '../composables/useRedisConnection.js'
import { useRedisKeys } from '../composables/useRedisKeys.js'
import { useRedisBackup } from '../composables/useRedisBackup.js'
import RedisKeyTree from './RedisKeyTree.vue'
import RedisKeyList from './RedisKeyList.vue'
import RedisValueView from './RedisValueView.vue'
import RedisRestoreDialog from './RedisRestoreDialog.vue'
import PanelHeader from './PanelHeader.vue'
import IconButton from './IconButton.vue'

const props = defineProps({
  hostId: { type: Number, required: true },
})

const store = useConnectionStore()
const listeners = useListenerGroup()

const host = computed(() => store.hosts.find(h => h.id === props.hostId))
// Stays null until the host is loaded and actually has a Redis URI, so nothing
// below fires against a half-built tab.
const hostId = computed(() => (host.value?.redis_uri ? props.hostId : null))

const displayUri = computed(() => redisDisplayUri(host.value?.redis_uri))
const tunnelHostName = computed(
  () => store.hosts.find(h => h.id === host.value?.redis_tunnel_host_id)?.name || 'SSH',
)

const activeDb = ref(0)
const connection = useRedisConnection(hostId)
const { info: serverInfo, connecting, error: connectError } = connection

const keys = useRedisKeys(hostId, activeDb, serverInfo)
const {
  pattern,
  typeFilter,
  withMemory,
  keys: keyPage,
  loading: keysLoading,
  error: keysError,
  done: keysDone,
  hasNext,
  canGoBack,
  pageNumber,
  summary: keysSummary,
} = keys

const connectionName = computed(() => host.value?.name || '')
const backup = useRedisBackup(hostId, activeDb, connectionName, refreshAll)
const {
  busy,
  currentAction,
  progress,
  restoreConfirm,
} = backup

const isCluster = computed(() => serverInfo.value?.mode === 'cluster')
const databases = computed(() => {
  const dbs = serverInfo.value?.databases ?? []
  // A server with no keys reports no databases at all; db0 always exists.
  return dbs.length > 0 ? dbs : [{ index: 0, keys: 0, expires: 0 }]
})

const tree = ref({ groups: [], scanned: 0, truncated: false })
const treeLoading = ref(false)
const activeGroup = ref(null)

const activeKey = ref(null)
const valuePage = ref(null)
const valueLoading = ref(false)
const valueError = ref('')
const valueOffset = ref(0)

const activeKeyLabel = computed(() =>
  activeKey.value?.key.binary
    ? `⟨binary ${activeKey.value.key.bytes} bytes⟩`
    : (activeKey.value?.key.text ?? ''),
)

async function connect() {
  await connection.connect()
  if (serverInfo.value) {
    activeDb.value = databases.value[0]?.index ?? 0
    await Promise.all([keys.restart(), loadTree()])
  }
}

async function loadTree() {
  if (!hostId.value) return
  treeLoading.value = true
  try {
    tree.value = await invoke('redis_key_tree', {
      hostId: hostId.value,
      db: activeDb.value,
      pattern: null,
    })
  } catch (err) {
    toast(`Could not group the keys: ${err}`, 'error')
    tree.value = { groups: [], scanned: 0, truncated: false }
  } finally {
    treeLoading.value = false
  }
}

async function refreshAll() {
  await connection.refresh()
  await Promise.all([keys.restart(), loadTree()])
}

async function selectDb(index) {
  if (index === activeDb.value) return
  activeDb.value = index
  activeGroup.value = null
  activeKey.value = null
  valuePage.value = null
  pattern.value = ''
  await Promise.all([keys.restart(), loadTree()])
}

/**
 * Narrow the *server-side* scan, not just the rendered list. Clicking a group
 * is only useful if it changes what Redis iterates.
 */
async function selectGroup(group) {
  if (group.binary) {
    toast('That prefix is not text, so it cannot be used as a match pattern', 'warning')
    return
  }
  activeGroup.value = group.prefix
  pattern.value = prefixPattern(group.prefix)
  await keys.restart()
}

async function onSearch() {
  activeGroup.value = null
  await keys.restart()
}

async function openKey(key) {
  activeKey.value = key
  valueOffset.value = 0
  await loadValue('0', 0, false)
}

async function loadValue(cursor, offset, append) {
  if (!hostId.value || !activeKey.value) return
  valueLoading.value = true
  valueError.value = ''
  try {
    const page = await invoke('redis_key_value', {
      hostId: hostId.value,
      db: activeDb.value,
      keyB64: activeKey.value.key_b64,
      cursor,
      offset,
      limit: MAX_ELEMENTS_PER_PAGE,
    })
    valuePage.value =
      append && valuePage.value
        ? { ...page, entries: [...valuePage.value.entries, ...page.entries] }
        : page
  } catch (err) {
    valueError.value = String(err)
  } finally {
    valueLoading.value = false
  }
}

/**
 * Hash and set page by cursor; list and sorted set page by index. The backend
 * takes both and ignores the one that does not apply to the type.
 */
async function loadMoreValue() {
  if (!valuePage.value) return
  const byCursor = ['hash', 'set'].includes(valuePage.value.kind)
  if (byCursor) {
    if (valuePage.value.done) return
    await loadValue(valuePage.value.cursor, 0, true)
  } else {
    valueOffset.value += valuePage.value.entries.length
    await loadValue('0', valueOffset.value, true)
  }
}

onMounted(async () => {
  await listeners.listen('redis-op-progress', e => backup.applyProgress(e.payload))
  await listeners.listen('redis-op-cancelled', e => backup.applyCancelled(e.payload))
  await connect()
})

// The component is reused across hosts, so everything host-specific resets.
watch(
  () => props.hostId,
  async () => {
    connection.reset()
    keys.reset()
    activeDb.value = 0
    activeGroup.value = null
    activeKey.value = null
    valuePage.value = null
    tree.value = { groups: [], scanned: 0, truncated: false }
    await connect()
  },
)
</script>
