<template>
  <!-- SSH Config Import Dialog -->
  <div
    v-if="showSshConfigDialog"
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
  >
    <div class="bg-[#252526] border border-[#3c3c3c] rounded-lg w-96 max-h-[80vh] flex flex-col shadow-xl">
      <div class="px-4 py-3 border-b border-[#3c3c3c] flex items-center justify-between">
        <h3 class="text-sm font-medium text-[#cccccc]">Import from ~/.ssh/config</h3>
        <button @click="showSshConfigDialog = false" class="text-[#858585] hover:text-[#cccccc]">×</button>
      </div>
      <div class="flex-1 overflow-y-auto p-2">
        <div
          v-for="(host, index) in sshConfigHosts"
          :key="index"
          class="flex items-center gap-2 px-2 py-1.5 rounded hover:bg-[#2a2d2e]"
        >
          <input
            type="checkbox"
            :checked="selectedSshHosts.has(index)"
            @change="(e) => e.target.checked ? selectedSshHosts.add(index) : selectedSshHosts.delete(index)"
            class="accent-[#007acc]"
          />
          <div class="flex-1 min-w-0">
            <div class="text-xs text-[#cccccc] truncate">{{ host.name }}</div>
            <div class="text-[10px] text-[#858585] truncate">{{ host.username }}@{{ host.host }}:{{ host.port }} · {{ host.auth_type }}</div>
          </div>
        </div>
      </div>
      <div class="px-4 py-3 border-t border-[#3c3c3c] flex justify-end gap-2">
        <button
          @click="showSshConfigDialog = false"
          class="px-3 py-1.5 text-xs text-[#cccccc] hover:bg-[#3c3c3c] rounded"
        >
          Cancel
        </button>
        <button
          @click="confirmSshConfigImport"
          class="px-3 py-1.5 text-xs bg-[#0e639c] hover:bg-[#1177bb] text-white rounded"
        >
          Import {{ selectedSshHosts.size }} host{{ selectedSshHosts.size === 1 ? '' : 's' }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref } from 'vue'
import { useConnectionStore } from '../stores/connection.js'
import { invoke } from '../utils/invoke.js'
import { toast } from '../utils/toast.js'
import { normalizeImportHost, summarizeImport } from '../utils/hostImport.js'

/**
 * Lists the hosts found in ~/.ssh/config with checkboxes and imports the
 * selected ones. The parent calls open().
 */
const store = useConnectionStore()

const showSshConfigDialog = ref(false)
const sshConfigHosts = ref([])
const selectedSshHosts = ref(new Set())

async function open() {
  try {
    const hosts = await invoke('parse_ssh_config')
    if (!hosts || hosts.length === 0) {
      toast('No hosts found in ~/.ssh/config', 'warning')
      return
    }
    sshConfigHosts.value = hosts
    selectedSshHosts.value = new Set(hosts.map((_, i) => i))
    showSshConfigDialog.value = true
  } catch (err) {
    toast('Failed to parse SSH config: ' + err, 'error')
  }
}

async function confirmSshConfigImport() {
  const toImport = sshConfigHosts.value.filter((_, i) => selectedSshHosts.value.has(i))
  if (toImport.length === 0) {
    showSshConfigDialog.value = false
    return
  }
  try {
    const entries = toImport.map((raw) => ({
      host: normalizeImportHost(raw),
      replace_id: null,
    }))
    const summary = await store.importHosts(entries)
    showSshConfigDialog.value = false
    const { message, type } = summarizeImport(summary)
    toast(message, type)
  } catch (err) {
    toast('Import failed: ' + err, 'error')
  }
}

defineExpose({ open })
</script>
