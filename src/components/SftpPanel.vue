<template>
  <div class="w-72 h-full bg-[#252526] border-l border-[#3c3c3c] flex flex-col relative">
    <!-- Path breadcrumbs -->
    <div class="px-2 py-1.5 border-b border-[#3c3c3c] flex items-center gap-0.5 text-xs overflow-x-auto whitespace-nowrap">
      <span
        v-for="(segment, index) in breadcrumbs"
        :key="index"
        class="flex items-center gap-0.5 shrink-0"
      >
        <button
          v-if="index === 0"
          @click="navigateTo(segment.path)"
          class="text-[#858585] hover:text-[#cccccc] p-0.5 rounded"
          title="Go to root"
        >
          <Home :size="12" />
        </button>
        <button
          v-else
          @click="navigateTo(segment.path)"
          class="text-[#858585] hover:text-[#cccccc] px-0.5 rounded"
          :class="index === breadcrumbs.length - 1 ? 'text-[#cccccc] font-medium cursor-default' : ''"
        >
          {{ segment.name }}
        </button>
        <ChevronRight
          v-if="index < breadcrumbs.length - 1"
          :size="10"
          class="text-[#6e6e6e] shrink-0"
        />
      </span>
    </div>

    <!-- Toolbar -->
    <div class="px-2 py-1.5 border-b border-[#3c3c3c] flex gap-1.5 flex-wrap items-center">
      <button @click="goUp" class="text-xs bg-[#3c3c3c] hover:bg-[#37373d] text-[#cccccc] px-1.5 py-0.5 rounded">↑ Up</button>
      <button @click="onUpload" class="text-xs bg-[#0e639c] hover:bg-[#1177bb] text-white px-1.5 py-0.5 rounded">Upload</button>
      <button @click="onMkdir" class="text-xs bg-[#3c3c3c] hover:bg-[#37373d] text-[#cccccc] px-1.5 py-0.5 rounded">+ Folder</button>
      <button @click="loadFiles" class="text-xs bg-[#3c3c3c] hover:bg-[#37373d] text-[#cccccc] px-1.5 py-0.5 rounded">↻</button>
      <div class="relative ml-auto">
        <button
          @click.stop="showColumnMenu = !showColumnMenu"
          class="text-xs text-[#858585] hover:text-[#cccccc] px-1.5 py-0.5"
          title="Toggle columns"
        >
          ☰
        </button>
        <div
          v-if="showColumnMenu"
          @click.stop
          class="absolute right-0 top-full mt-0.5 bg-[#252526] border border-[#3c3c3c] rounded shadow-lg py-1 z-50 min-w-[7rem]"
        >
          <label class="flex items-center gap-1.5 px-2 py-0.5 text-xs text-[#cccccc] cursor-pointer hover:bg-[#2a2d2e]">
            <input v-model="showColumns.size" type="checkbox" class="accent-[#007acc]" />
            Size
          </label>
          <label class="flex items-center gap-1.5 px-2 py-0.5 text-xs text-[#cccccc] cursor-pointer hover:bg-[#2a2d2e]">
            <input v-model="showColumns.modified" type="checkbox" class="accent-[#007acc]" />
            Modified
          </label>
          <label class="flex items-center gap-1.5 px-2 py-0.5 text-xs text-[#cccccc] cursor-pointer hover:bg-[#2a2d2e]">
            <input v-model="showColumns.perms" type="checkbox" class="accent-[#007acc]" />
            Perms
          </label>
        </div>
      </div>
    </div>

    <!-- Column headers -->
    <div class="px-2 py-0.5 border-b border-[#3c3c3c] flex items-center text-xs text-[#858585] select-none">
      <span class="flex-1 min-w-0 cursor-pointer hover:text-[#cccccc]" @click="setSort('name')">
        Name {{ sortIndicator('name') }}
      </span>
      <span v-if="showColumns.size" class="w-12 shrink-0 text-right cursor-pointer hover:text-[#cccccc]" @click="setSort('size')">
        Size {{ sortIndicator('size') }}
      </span>
      <span v-if="showColumns.modified" class="w-14 shrink-0 text-right cursor-pointer hover:text-[#cccccc] ml-1.5" @click="setSort('modified')">
        Modified {{ sortIndicator('modified') }}
      </span>
      <span v-if="showColumns.perms" class="w-16 shrink-0 text-right ml-1.5">Perms</span>
    </div>

    <!-- Quick filter -->
    <div class="px-2 py-1 border-b border-[#3c3c3c]">
      <input
        v-model="filterQuery"
        type="text"
        placeholder="Filter files..."
        class="w-full bg-[#3c3c3c] border border-[#3c3c3c] rounded px-2 py-0.5 text-xs text-[#cccccc] focus:outline-none focus:border-[#007acc] placeholder-[#6e6e6e]"
      />
    </div>

    <!-- Edited files toolbar -->
    <div
      v-if="editingFiles.size > 0"
      class="px-2 py-1 border-b border-[#3c3c3c] bg-[#cca700]/20 flex items-center justify-between"
    >
      <span class="text-xs text-[#cca700]">{{ editingFiles.size }} file{{ editingFiles.size > 1 ? 's' : '' }} being edited</span>
      <button @click="uploadAllEdits" class="text-xs bg-[#cca700] hover:bg-[#ffd700] text-black px-2 py-0.5 rounded font-medium">Upload All</button>
    </div>

    <!-- File list -->
    <div class="flex-1 overflow-y-auto relative">
      <div
        v-if="loading"
        class="flex items-center justify-center h-20 text-[#6e6e6e] text-sm"
      >
        Loading...
      </div>
      <div v-else-if="filteredFiles.length === 0" class="flex items-center justify-center h-20 text-[#6e6e6e] text-sm">
        {{ sortedFiles.length === 0 ? 'Empty directory' : 'No matching files' }}
      </div>
      <div v-else>
        <div
          v-for="(file, index) in filteredFiles"
          :key="file.path"
          class="flex items-center px-2 py-0.5 hover:bg-[#2a2d2e] cursor-pointer text-sm"
          :class="[
            file.is_dir ? 'text-[#007acc]' : 'text-[#cccccc]',
            selectedFiles.has(file.path) ? 'bg-[#094771]' : ''
          ]"
          @click="handleFileClick(file, index, $event)"
          @dblclick="file.is_dir ? navigateTo(file.path) : onPreviewFile(file)"
          @contextmenu.prevent="showContextMenu($event, file)"
        >
          <input
            type="checkbox"
            :checked="selectedFiles.has(file.path)"
            class="accent-[#007acc] mr-1.5 shrink-0"
            @click.stop
            @change="handleFileClick(file, index, { ctrlKey: true })"
          />
          <Folder v-if="file.is_dir" :size="12" class="shrink-0 mr-1.5" />
          <FileText v-else :size="12" class="shrink-0 mr-1.5 text-[#6e6e6e]" />
          <span class="truncate flex-1 min-w-0">{{ file.name }}</span>
          <span v-if="showColumns.size" class="w-12 shrink-0 text-right text-xs text-[#6e6e6e]">{{ file.is_dir ? '-' : formatSize(file.size) }}</span>
          <span v-if="showColumns.modified" class="w-14 shrink-0 text-right text-xs text-[#6e6e6e] ml-1.5">{{ formatDate(file.modified) }}</span>
          <span v-if="showColumns.perms" class="w-16 shrink-0 text-right text-xs text-[#6e6e6e] ml-1.5 font-mono">{{ formatPermissions(file.permissions, file.is_dir) }}</span>
        </div>
      </div>
    </div>

    <!-- Context menu -->
    <div
      v-if="contextMenu.show"
      ref="contextMenuEl"
      class="fixed bg-[#252526] border border-[#3c3c3c] rounded shadow-lg py-1 z-50 min-w-[8rem]"
      :style="{ top: contextMenu.y + 'px', left: contextMenu.x + 'px' }"
    >
      <!-- Multi-selection mode -->
      <template v-if="contextMenu.multi">
        <button @click="onBulkDownloadFromMenu" class="block w-full text-left px-4 py-1.5 text-sm text-[#cccccc] hover:bg-[#2a2d2e]">Download Selected ({{ selectedFiles.size }})</button>
        <button @click="onBulkDeleteFromMenu" class="block w-full text-left px-4 py-1.5 text-sm text-[#f44336] hover:bg-[#2a2d2e]">Delete Selected ({{ selectedFiles.size }})</button>
        <button @click="clearSelection(); contextMenu.show = false" class="block w-full text-left px-4 py-1.5 text-sm text-[#858585] hover:bg-[#2a2d2e]">Clear Selection</button>
      </template>
      <!-- Single-file mode -->
      <template v-else>
        <button v-if="contextMenu.file && !contextMenu.file.is_dir" @click="onPreview" class="block w-full text-left px-4 py-1.5 text-sm text-[#cccccc] hover:bg-[#2a2d2e]">Preview</button>
        <button v-if="contextMenu.file && !contextMenu.file.is_dir" @click="onEdit" class="block w-full text-left px-4 py-1.5 text-sm text-[#cccccc] hover:bg-[#2a2d2e]">Edit</button>
        <button v-if="contextMenu.file && contextMenu.file.is_dir" @click="onDownloadDir" class="block w-full text-left px-4 py-1.5 text-sm text-[#cccccc] hover:bg-[#2a2d2e]">Download Folder</button>
        <button v-if="contextMenu.file && !contextMenu.file.is_dir" @click="onDownload" class="block w-full text-left px-4 py-1.5 text-sm text-[#cccccc] hover:bg-[#2a2d2e]">Download</button>
        <button @click="copyRemotePath" class="block w-full text-left px-4 py-1.5 text-sm text-[#cccccc] hover:bg-[#2a2d2e]">Copy Path</button>
        <button @click="onRename" class="block w-full text-left px-4 py-1.5 text-sm text-[#cccccc] hover:bg-[#2a2d2e]">Rename</button>
        <button @click="onDelete" class="block w-full text-left px-4 py-1.5 text-sm text-[#f44336] hover:bg-[#2a2d2e]">Delete</button>
      </template>
    </div>

    <ConfirmDialog
      :show="confirmDialog.show"
      :title="confirmDialog.title"
      :message="confirmDialog.message"
      :danger="confirmDialog.danger"
      confirm-text="Delete"
      @confirm="confirmDialog.onConfirm"
      @cancel="confirmDialog.show = false"
    />

    <PromptDialog
      :show="promptDialog.show"
      :title="promptDialog.title"
      :message="promptDialog.message"
      :placeholder="promptDialog.placeholder"
      :default-value="promptDialog.defaultValue"
      @confirm="promptDialog.onConfirm"
      @cancel="promptDialog.show = false"
    />

    <!-- File preview modal -->
    <div
      v-if="previewModal.show"
      class="fixed inset-0 bg-black/50 flex items-center justify-center z-50"
      @click.self="previewModal.show = false"
    >
      <div class="bg-[#252526] border border-[#3c3c3c] rounded shadow-xl w-[40rem] h-[30rem] flex flex-col max-w-[90vw] max-h-[80vh]">
        <div class="flex items-center justify-between px-3 py-2 border-b border-[#3c3c3c]">
          <span class="text-xs text-[#cccccc]">Preview: {{ previewModal.fileName }}</span>
          <button @click="previewModal.show = false" class="text-[#858585] hover:text-[#cccccc]">×</button>
        </div>
        <div class="flex-1 overflow-auto p-3">
          <div v-if="previewModal.loading" class="flex items-center justify-center h-full text-[#858585] text-sm">
            Loading...
          </div>
          <pre v-else class="text-[11px] text-[#cccccc] font-mono whitespace-pre-wrap break-all">{{ previewModal.content }}</pre>
        </div>
      </div>
    </div>

    <!-- Progress toasts -->
    <div v-if="transfers.length > 0" class="border-t border-[#3c3c3c] p-2 space-y-2">
      <div v-for="t in transfers" :key="t.file" class="text-xs">
        <div class="flex justify-between text-[#858585] mb-1">
          <span class="truncate">{{ t.file }}</span>
          <span>{{ Math.round((t.bytes / t.total) * 100) }}%</span>
        </div>
        <div class="h-1 bg-[#3c3c3c] rounded overflow-hidden">
          <div class="h-full bg-[#007acc] transition-all" :style="{ width: (t.bytes / t.total) * 100 + '%' }"></div>
        </div>
      </div>
    </div>

  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from 'vue'
import { useConnectionStore } from '../stores/connection.js'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { openPath } from '@tauri-apps/plugin-opener'
import { Folder, FileText, Home, ChevronRight } from 'lucide-vue-next'
import ConfirmDialog from './ConfirmDialog.vue'
import PromptDialog from './PromptDialog.vue'

const props = defineProps({
  sftpSessionId: {
    type: String,
    required: true,
  },
})

const store = useConnectionStore()
const currentPath = ref('/')
const files = ref([])
const loading = ref(false)
const contextMenu = ref({ show: false, x: 0, y: 0, file: null })
const contextMenuEl = ref(null)
const transfers = ref([])
const sortKey = ref('name')
const sortOrder = ref('asc')
const showColumnMenu = ref(false)
const showColumns = ref({
  size: true,
  modified: true,
  perms: true,
})
const filterQuery = ref('')
const selectedFiles = ref(new Set())
const lastSelectedIndex = ref(-1)
let unlistenProgress = null
let unlistenFileDrop = null

const confirmDialog = ref({
  show: false,
  title: '',
  message: '',
  danger: false,
  onConfirm: () => {},
})

const promptDialog = ref({
  show: false,
  title: '',
  message: '',
  placeholder: '',
  defaultValue: '',
  onConfirm: () => {},
})

const previewModal = ref({
  show: false,
  fileName: '',
  content: '',
  loading: false,
})

const editingFiles = ref(new Map()) // localPath -> { remotePath, lastModified, intervalId }

function openConfirm(options) {
  confirmDialog.value = {
    show: true,
    title: options.title || 'Confirm',
    message: options.message || '',
    danger: options.danger || false,
    onConfirm: () => {
      confirmDialog.value.show = false
      options.onConfirm()
    },
  }
}

function openPrompt(options) {
  promptDialog.value = {
    show: true,
    title: options.title || 'Prompt',
    message: options.message || '',
    placeholder: options.placeholder || '',
    defaultValue: options.defaultValue || '',
    onConfirm: (value) => {
      promptDialog.value.show = false
      options.onConfirm(value)
    },
  }
}

function showToast(message, type = 'success') {
  window.dispatchEvent(new CustomEvent('app-toast', { detail: { message, type } }))
}

const breadcrumbs = computed(() => {
  const parts = currentPath.value.split('/').filter(Boolean)
  const result = [{ name: 'root', path: '/' }]
  let path = ''
  for (const part of parts) {
    path += '/' + part
    result.push({ name: part, path })
  }
  return result
})

const sortedFiles = computed(() => {
  const list = [...files.value]
  list.sort((a, b) => {
    // Directories always first
    if (a.is_dir !== b.is_dir) {
      return a.is_dir ? -1 : 1
    }
    let cmp = 0
    switch (sortKey.value) {
      case 'size':
        cmp = (a.size || 0) - (b.size || 0)
        break
      case 'modified':
        cmp = (a.modified || 0) - (b.modified || 0)
        break
      case 'name':
      default:
        cmp = a.name.localeCompare(b.name)
        break
    }
    return sortOrder.value === 'asc' ? cmp : -cmp
  })
  return list
})

const filteredFiles = computed(() => {
  const q = filterQuery.value.trim().toLowerCase()
  if (!q) return sortedFiles.value
  return sortedFiles.value.filter(f => f.name.toLowerCase().includes(q))
})

function setSort(key) {
  if (sortKey.value === key) {
    sortOrder.value = sortOrder.value === 'asc' ? 'desc' : 'asc'
  } else {
    sortKey.value = key
    sortOrder.value = 'asc'
  }
}

function sortIndicator(key) {
  if (sortKey.value !== key) return ''
  return sortOrder.value === 'asc' ? '▲' : '▼'
}

function formatPermissions(perm, isDir) {
  if (perm == null) return '----------'
  const type = isDir ? 'd' : '-'
  const r = (bit) => (perm & bit) ? 'r' : '-'
  const w = (bit) => (perm & bit) ? 'w' : '-'
  const x = (bit) => (perm & bit) ? 'x' : '-'
  return type +
    r(0o400) + w(0o200) + x(0o100) +
    r(0o040) + w(0o020) + x(0o010) +
    r(0o004) + w(0o002) + x(0o001)
}

function formatDate(timestamp) {
  if (!timestamp) return '-'
  const date = new Date(timestamp * 1000)
  const now = new Date()
  const isSameYear = date.getFullYear() === now.getFullYear()
  if (isSameYear) {
    return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
  }
  return date.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' })
}

async function resolveHomeDir() {
  if (!props.sftpSessionId) return
  try {
    const home = await invoke('sftp_realpath', { sftpSessionId: props.sftpSessionId, remotePath: '.' })
    if (home) currentPath.value = home
  } catch (e) {
    console.warn('Failed to resolve home dir, falling back to /:', e)
    currentPath.value = '/'
  }
}

async function uploadDroppedFiles(paths) {
  if (!paths || paths.length === 0) return
  const dirPath = currentPath.value === '/' ? '' : currentPath.value
  for (const localPath of paths) {
    const fileName = localPath.split(/[\\/]/).pop()
    if (!fileName) continue
    const fullRemotePath = dirPath ? `${dirPath}/${fileName}` : fileName
    try {
      await invoke('sftp_upload', {
        sftpSessionId: props.sftpSessionId,
        localPath,
        remotePath: fullRemotePath,
      })
      showToast(`Uploaded ${fileName}`, 'success')
    } catch (e) {
      console.error('Upload failed:', e)
      showToast(`Upload failed for ${fileName}: ${e}`, 'error')
    }
  }
  await loadFiles()
}

function closeMenu() {
  contextMenu.value.show = false
  showColumnMenu.value = false
}

function clearSelection() {
  selectedFiles.value.clear()
  lastSelectedIndex.value = -1
}

function handleFileClick(file, index, event) {
  if (event.ctrlKey || event.metaKey) {
    // Ctrl/Cmd + click: toggle
    if (selectedFiles.value.has(file.path)) {
      selectedFiles.value.delete(file.path)
    } else {
      selectedFiles.value.add(file.path)
    }
    lastSelectedIndex.value = index
  } else if (event.shiftKey && lastSelectedIndex.value >= 0) {
    // Shift + click: range select
    const start = Math.min(lastSelectedIndex.value, index)
    const end = Math.max(lastSelectedIndex.value, index)
    for (let i = start; i <= end; i++) {
      selectedFiles.value.add(filteredFiles.value[i].path)
    }
  } else {
    // Plain click: select single, clear others
    clearSelection()
    selectedFiles.value.add(file.path)
    lastSelectedIndex.value = index
  }
}

async function bulkDelete() {
  const paths = Array.from(selectedFiles.value)
  const count = paths.length
  openConfirm({
    title: 'Delete Selected',
    message: `Delete ${count} selected item${count > 1 ? 's' : ''}? This cannot be undone.`,
    danger: true,
    onConfirm: async () => {
      let deleted = 0
      let failed = 0
      for (const path of paths) {
        const file = files.value.find(f => f.path === path)
        if (!file) continue
        try {
          if (file.is_dir) {
            await store.sftpRmdir(props.sftpSessionId, path)
          } else {
            await store.sftpDelete(props.sftpSessionId, path)
          }
          deleted++
        } catch (e) {
          console.error('Delete failed:', e)
          failed++
        }
      }
      clearSelection()
      await loadFiles()
      if (failed > 0) {
        showToast(`Deleted ${deleted}, failed ${failed}`, 'warning')
      } else {
        showToast(`Deleted ${deleted} item${deleted > 1 ? 's' : ''}`, 'success')
      }
    },
  })
}

async function uploadAllEdits() {
  for (const [localPath, edit] of editingFiles.value) {
    try {
      await invoke('sftp_upload', {
        sftpSessionId: props.sftpSessionId,
        localPath,
        remotePath: edit.remotePath,
      })
      showToast(`Uploaded ${edit.fileName}`, 'success')
    } catch (e) {
      console.error('Upload edit failed:', e)
      showToast(`Upload failed for ${edit.fileName}: ${e}`, 'error')
    }
  }
  editingFiles.value.clear()
}

async function bulkDownload() {
  const paths = Array.from(selectedFiles.value)
  let completed = 0
  let failed = 0
  for (const path of paths) {
    const file = files.value.find(f => f.path === path)
    if (!file || file.is_dir) continue
    try {
      await store.sftpDownload(props.sftpSessionId, path)
      completed++
    } catch (e) {
      console.error('Download failed:', e)
      failed++
    }
  }
  clearSelection()
  if (failed > 0) {
    showToast(`Downloaded ${completed}, failed ${failed}`, 'warning')
  } else {
    showToast(`Downloaded ${completed} file${completed > 1 ? 's' : ''}`, 'success')
  }
}

onMounted(async () => {
  // Load saved column visibility
  const saved = localStorage.getItem('sftp-columns')
  if (saved) {
    try {
      showColumns.value = { ...showColumns.value, ...JSON.parse(saved) }
    } catch (e) {
      console.warn('Failed to parse saved column visibility:', e)
    }
  }
  await resolveHomeDir()
  await loadFiles()
  unlistenProgress = await listen('sftp-progress', (event) => {
    const p = event.payload
    const existing = transfers.value.find(t => t.file === p.file)
    if (existing) {
      existing.bytes = p.bytes_transferred
      existing.total = p.total_bytes
      if (existing.bytes >= existing.total) {
        setTimeout(() => {
          transfers.value = transfers.value.filter(t => t.file !== p.file)
        }, 2000)
      }
    } else {
      transfers.value.push({
        file: p.file,
        bytes: p.bytes_transferred,
        total: p.total_bytes,
      })
    }
  })
  unlistenFileDrop = await listen('tauri://drag-drop', (event) => {
    const payload = event.payload
    const paths = payload?.paths
    if (paths && paths.length > 0) {
      uploadDroppedFiles(paths)
    }
  })
  window.addEventListener('click', closeMenu)
  window.addEventListener('contextmenu', closeMenu, true)
})

onUnmounted(() => {
  if (unlistenProgress) unlistenProgress()
  if (unlistenFileDrop) unlistenFileDrop()
  window.removeEventListener('click', closeMenu)
  window.removeEventListener('contextmenu', closeMenu, true)
  // Stop all edit polling intervals
  editingFiles.value.clear()
})

watch(() => props.sftpSessionId, async () => {
  await resolveHomeDir()
  await loadFiles()
})

watch(showColumns, (val) => {
  localStorage.setItem('sftp-columns', JSON.stringify(val))
}, { deep: true })

async function loadFiles() {
  if (!props.sftpSessionId) return
  loading.value = true
  try {
    const result = await store.sftpList(props.sftpSessionId, currentPath.value)
    files.value = result || []
  } catch (e) {
    console.error('sftp_list failed:', e)
  }
  loading.value = false
}

function navigateTo(path) {
  clearSelection()
  filterQuery.value = ''
  currentPath.value = path
  loadFiles()
}

function goUp() {
  if (currentPath.value === '/') return
  clearSelection()
  filterQuery.value = ''
  const parts = currentPath.value.split('/').filter(Boolean)
  parts.pop()
  currentPath.value = parts.length === 0 ? '/' : '/' + parts.join('/')
  loadFiles()
}

async function onUpload() {
  const remotePath = currentPath.value === '/' ? '' : currentPath.value
  const uploaded = await store.sftpUpload(props.sftpSessionId, remotePath)
  if (uploaded) {
    await loadFiles()
  }
}

async function onDownload() {
  const file = contextMenu.value.file
  if (!file || file.is_dir) return
  contextMenu.value.show = false
  try {
    const savedPath = await store.sftpDownload(props.sftpSessionId, file.path)
    showToast(`Downloaded to ${savedPath}`, 'success')
  } catch (e) {
    console.error('Download failed:', e)
    showToast('Download failed: ' + e, 'error')
  }
}

async function onDownloadDir() {
  const file = contextMenu.value.file
  if (!file || !file.is_dir) return
  contextMenu.value.show = false
  try {
    const savedPath = await invoke('sftp_download_dir', { sftpSessionId: props.sftpSessionId, remotePath: file.path })
    showToast(`Downloaded folder to ${savedPath}`, 'success')
  } catch (e) {
    console.error('Download folder failed:', e)
    showToast('Download folder failed: ' + e, 'error')
  }
}

function onDelete() {
  const file = contextMenu.value.file
  if (!file) return
  contextMenu.value.show = false
  const isDir = file.is_dir
  openConfirm({
    title: isDir ? 'Delete Folder' : 'Delete File',
    message: isDir
      ? `Delete "${file.name}" and all its contents? This cannot be undone.`
      : `Delete "${file.name}"? This cannot be undone.`,
    danger: true,
    onConfirm: async () => {
      try {
        if (isDir) {
          await store.sftpRmdir(props.sftpSessionId, file.path)
        } else {
          await store.sftpDelete(props.sftpSessionId, file.path)
        }
        await loadFiles()
        showToast(isDir ? `Deleted folder "${file.name}"` : `Deleted "${file.name}"`, 'success')
      } catch (e) {
        console.error('Delete failed:', e)
        showToast('Delete failed: ' + e, 'error')
      }
    },
  })
}

async function onMkdir() {
  openPrompt({
    title: 'New Folder',
    message: 'Enter a name for the new folder:',
    placeholder: 'folder-name',
    onConfirm: async (name) => {
      const dirPath = currentPath.value === '/' ? '' : currentPath.value
      const fullPath = dirPath ? `${dirPath}/${name}` : name
      try {
        await store.sftpMkdir(props.sftpSessionId, fullPath)
        await loadFiles()
        showToast(`Created folder "${name}"`, 'success')
      } catch (e) {
        console.error('mkdir failed:', e)
        showToast('Failed to create folder: ' + e, 'error')
      }
    },
  })
}

async function copyRemotePath() {
  const file = contextMenu.value.file
  if (!file) return
  contextMenu.value.show = false
  try {
    await navigator.clipboard.writeText(file.path)
    showToast('Path copied to clipboard', 'success')
  } catch (e) {
    console.warn('Copy path failed:', e)
    showToast('Failed to copy path', 'error')
  }
}

async function onPreviewFile(file) {
  if (!file || file.is_dir) return
  previewModal.value = { show: true, fileName: file.name, content: '', loading: true }
  try {
    const content = await invoke('sftp_read_file', { sftpSessionId: props.sftpSessionId, remotePath: file.path })
    previewModal.value.content = content
  } catch (e) {
    console.error('Preview failed:', e)
    previewModal.value.content = `Error: ${e}`
  } finally {
    previewModal.value.loading = false
  }
}

async function onPreview() {
  const file = contextMenu.value.file
  if (!file || file.is_dir) return
  contextMenu.value.show = false
  await onPreviewFile(file)
}

async function onEdit() {
  const file = contextMenu.value.file
  if (!file || file.is_dir) return
  contextMenu.value.show = false
  try {
    const localPath = await invoke('sftp_edit_file', {
      sftpSessionId: props.sftpSessionId,
      remotePath: file.path,
    })
    // Open in system editor
    await openPath(localPath)
    showToast(`Opened ${file.name} in editor`, 'success')

    // Start polling for changes
    const startTime = Math.floor(Date.now() / 1000)
    editingFiles.value.set(localPath, {
      remotePath: file.path,
      lastModified: startTime,
      fileName: file.name,
    })

    // Poll every 3 seconds
    const intervalId = setInterval(async () => {
      const edit = editingFiles.value.get(localPath)
      if (!edit) {
        clearInterval(intervalId)
        return
      }
      try {
        const newMtime = await invoke('check_file_modified', {
          localPath,
          lastModified: edit.lastModified,
        })
        if (newMtime) {
          edit.lastModified = newMtime
          // Show upload toast
          showToast(`"${edit.fileName}" modified. Use Upload to sync changes.`, 'info')
        }
      } catch (e) {
        // File might have been deleted, stop polling
        editingFiles.value.delete(localPath)
        clearInterval(intervalId)
      }
    }, 3000)
  } catch (e) {
    console.error('Edit failed:', e)
    showToast('Edit failed: ' + e, 'error')
  }
}

async function onRename() {
  const file = contextMenu.value.file
  if (!file) return
  contextMenu.value.show = false
  openPrompt({
    title: 'Rename',
    message: `Rename "${file.name}" to:`,
    placeholder: 'new-name',
    defaultValue: file.name,
    onConfirm: async (newName) => {
      if (!newName || newName === file.name) return
      const parent = file.path.substring(0, file.path.lastIndexOf('/')) || '/'
      const newPath = parent === '/' ? '/' + newName : parent + '/' + newName
      try {
        await store.sftpRename(props.sftpSessionId, file.path, newPath)
        await loadFiles()
        showToast(`Renamed to "${newName}"`, 'success')
      } catch (e) {
        console.error('Rename failed:', e)
        showToast('Rename failed: ' + e, 'error')
      }
    },
  })
}

async function showContextMenu(event, file) {
  // If right-clicking a file that's not in the current selection,
  // and there are other files selected, clear and select only this one
  const isMulti = selectedFiles.value.size > 1 && selectedFiles.value.has(file.path)
  if (!isMulti && selectedFiles.value.size > 0) {
    clearSelection()
    selectedFiles.value.add(file.path)
  }

  contextMenu.value = {
    show: true,
    x: event.clientX,
    y: event.clientY,
    file,
    multi: isMulti,
  }
  // Wait for DOM render, then adjust position if off-screen
  await nextTick()
  const el = contextMenuEl.value
  if (el) {
    const rect = el.getBoundingClientRect()
    const vw = window.innerWidth
    const vh = window.innerHeight
    let x = contextMenu.value.x
    let y = contextMenu.value.y
    if (x + rect.width > vw) x = vw - rect.width - 8
    if (y + rect.height > vh) y = vh - rect.height - 8
    if (x < 8) x = 8
    if (y < 8) y = 8
    contextMenu.value.x = x
    contextMenu.value.y = y
  }
}

function onBulkDownloadFromMenu() {
  contextMenu.value.show = false
  bulkDownload()
}

function onBulkDeleteFromMenu() {
  contextMenu.value.show = false
  bulkDelete()
}

function formatSize(bytes) {
  if (bytes < 1024) return bytes + ' B'
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB'
  if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + ' MB'
  return (bytes / (1024 * 1024 * 1024)).toFixed(1) + ' GB'
}
</script>
