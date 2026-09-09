<template>
  <div class="w-72 h-full bg-[#252526] border-l border-[#3c3c3c] flex flex-col relative">
    <!-- Path breadcrumbs -->
    <div
      class="px-2 py-1.5 border-b border-[#3c3c3c] flex items-center gap-0.5 text-xs overflow-x-auto whitespace-nowrap"
      style="scrollbar-width: thin; scrollbar-color: #3c3c3c transparent;"
    >
      <span
        v-for="(segment, index) in breadcrumbs"
        :key="index"
        class="flex items-center gap-0.5 shrink-0 min-w-0"
      >
        <button
          v-if="index === 0"
          @click="navigateTo(segment.path)"
          class="text-[#858585] hover:text-[#cccccc] p-0.5 rounded shrink-0"
          title="Go to root"
        >
          <Home :size="12" />
        </button>
        <button
          v-else
          @click="index < breadcrumbs.length - 1 && navigateTo(segment.path)"
          class="px-1 py-0.5 rounded truncate max-w-[120px]"
          :class="index === breadcrumbs.length - 1
            ? 'text-[#cccccc] font-medium cursor-default bg-[#3c3c3c]/50'
            : 'text-[#858585] hover:text-[#cccccc] hover:bg-[#3c3c3c]/30 cursor-pointer'"
          :title="segment.path"
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

    <!-- File list -->
    <div class="flex-1 overflow-y-auto relative">
      <div
        v-if="loading"
        class="flex items-center justify-center h-20 text-[#6e6e6e] text-sm"
      >
        Loading...
      </div>
      <div v-else-if="listError" class="flex flex-col items-center justify-center gap-2 py-6 px-3 text-center">
        <p class="text-xs text-[#f44336]">Could not list this directory</p>
        <p class="text-[10px] text-[#858585] break-words" :title="listError">{{ listError }}</p>
        <button
          @click="loadFiles"
          class="mt-1 px-3 py-1 bg-[#0e639c] hover:bg-[#1177bb] text-white text-xs rounded"
        >
          Retry
        </button>
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

    <!-- File preview dialog -->
    <FilePreviewDialog
      :visible="previewModal.show"
      :file-name="previewModal.fileName"
      :file-path="previewModal.filePath"
      :file-size="previewModal.fileSize"
      :sftp-session-id="props.sftpSessionId"
      @close="previewModal.show = false"
      @edit="previewEdit"
      @download="previewDownload"
    />

    <FloatingEditor ref="editorRef" :sftp-session-id="props.sftpSessionId" />

    <!-- Transfer progress -->
    <div v-if="transfers.length > 0" class="border-t border-[#3c3c3c] bg-[#1e1e1e]">
      <div class="px-2 py-1 text-[10px] text-[#6e6e6e] font-medium uppercase tracking-wider border-b border-[#3c3c3c]/50">
        Transfers ({{ transfers.length }})
      </div>
      <div class="p-2 space-y-2 max-h-32 overflow-y-auto">
        <div v-for="t in transfers" :key="t.file" class="text-xs">
          <div class="flex items-center justify-between text-[#858585] mb-0.5">
            <span class="truncate flex-1 min-w-0 mr-2" :title="t.file">{{ t.fileName }}</span>
            <span class="shrink-0 text-[#cccccc] font-medium">
              <template v-if="t.total === 0 && !t.done">
                <span class="inline-block w-3 h-3 border-2 border-[#007acc] border-t-transparent rounded-full animate-spin align-text-bottom"></span>
              </template>
              <template v-else>
                {{ t.total === 0 ? '100%' : Math.round((t.bytes / t.total) * 100) + '%' }}
              </template>
            </span>
          </div>
          <div class="flex items-center justify-between text-[10px] text-[#6e6e6e] mb-1">
            <span v-if="t.total === 0 && !t.done">Preparing archive...</span>
            <span v-else>{{ formatSize(t.bytes) }} / {{ formatSize(t.total) }}</span>
            <span v-if="!t.done && t.speed > 0" class="text-[#89d185]">{{ formatSpeed(t.speed) }}</span>
            <span v-else-if="t.done && !t.fileName.includes('failed')" class="text-[#007acc]">Done</span>
            <span v-else-if="t.done && t.fileName.includes('failed')" class="text-[#f44336]">Failed</span>
          </div>
          <div class="h-1.5 bg-[#3c3c3c] rounded overflow-hidden">
            <div
              v-if="t.total === 0 && !t.done"
              class="h-full rounded bg-[#007acc] animate-pulse"
              style="width: 100%"
            ></div>
            <div
              v-else
              class="h-full rounded transition-all duration-300"
              :class="t.done ? 'bg-[#89d185]' : 'bg-[#007acc]'"
              :style="{ width: t.total === 0 ? '100%' : Math.min(100, (t.bytes / t.total) * 100) + '%' }"
            ></div>
          </div>
        </div>
      </div>
    </div>

  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted, onUnmounted, onActivated, onDeactivated, shallowRef } from 'vue'
import { useConnectionStore } from '../stores/connection.js'
import { invoke } from '../utils/invoke.js'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { Folder, FileText, Home, ChevronRight } from 'lucide-vue-next'
import ConfirmDialog from './ConfirmDialog.vue'
import PromptDialog from './PromptDialog.vue'
import FilePreviewDialog from './FilePreviewDialog.vue'
import FloatingEditor from './FloatingEditor.vue'
import { formatBytes as formatSize, formatSpeed } from '../utils/format.js'
import { toast } from '../utils/toast.js'
import { useConfirmDialog } from '../composables/useConfirmDialog.js'
import { useContextMenu } from '../composables/useContextMenu.js'
import { useListenerGroup } from '../composables/useListenerGroup.js'
import { useSftpTransfers } from '../composables/useSftpTransfers.js'

const props = defineProps({
  sftpSessionId: {
    type: String,
    required: true,
  },
})

const store = useConnectionStore()
const currentPath = ref('/')
const files = shallowRef([]) // replaced wholesale by loadFiles; rows are never edited in place
const loading = ref(false)
const listError = ref('')
const contextMenuEl = ref(null)
const { contextMenu, openContextMenu } = useContextMenu(contextMenuEl, { file: null })
const sortKey = ref('name')
const sortOrder = ref('asc')
const showColumnMenu = ref(false)
const showColumns = ref({
  size: false,
  modified: false,
  perms: false,
})
const filterQuery = ref('')
const selectedFiles = ref(new Set())
const lastSelectedIndex = ref(-1)

const listeners = useListenerGroup()

// Panels live inside <KeepAlive>, so a hidden panel is deactivated rather than
// unmounted and its listeners stay registered. Without this flag a file
// dropped on the visible panel would also upload to every other host whose
// panel had ever been opened.
const isPanelActive = ref(true)
onActivated(() => { isPanelActive.value = true })
onDeactivated(() => { isPanelActive.value = false })
const { transfers, handleProgress, beginFolderTransfer, finishFolderTransfer } = useSftpTransfers()

/** Run `fn`, logging and toasting any error with the given labels. */
async function withErrorToast(logLabel, toastPrefix, fn) {
  try {
    await fn()
  } catch (e) {
    console.error(logLabel + ' failed:', e)
    toast(toastPrefix + e, 'error')
  }
}

const { confirmDialog, openConfirm } = useConfirmDialog()

const promptDialog = ref({
  show: false,
  title: '',
  message: '',
  placeholder: '',
  defaultValue: '',
  onConfirm: () => {},
})

const editorRef = ref(null)

const previewModal = ref({
  show: false,
  fileName: '',
  filePath: '',
  fileSize: 0,
})

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

const breadcrumbs = computed(() => {
  const normalized = currentPath.value.replace(/\\/g, '/').replace(/\/+/g, '/')
  const parts = normalized.split('/').filter(Boolean)
  const result = [{ name: 'root', path: '/' }]
  let path = ''
  for (const part of parts) {
    path += '/' + part
    result.push({ name: part || '/', path })
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
      toast(`Uploaded ${fileName}`, 'success')
    } catch (e) {
      console.error('Upload failed:', e)
      toast(`Upload failed for ${fileName}: ${e}`, 'error')
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
        toast(`Deleted ${deleted}, failed ${failed}`, 'warning')
      } else {
        toast(`Deleted ${deleted} item${deleted > 1 ? 's' : ''}`, 'success')
      }
    },
  })
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
    toast(`Downloaded ${completed}, failed ${failed}`, 'warning')
  } else {
    toast(`Downloaded ${completed} file${completed > 1 ? 's' : ''}`, 'success')
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
  await listeners.listen('sftp-progress', (event) => {
    if (event.payload?.sftp_session_id !== props.sftpSessionId) return
    handleProgress(event.payload)
  })
  await listeners.listen('tauri://drag-drop', (event) => {
    if (!isPanelActive.value) return
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
  window.removeEventListener('click', closeMenu)
  window.removeEventListener('contextmenu', closeMenu, true)
})

watch(() => props.sftpSessionId, async () => {
  await resolveHomeDir()
  await loadFiles()
})

watch(showColumns, (val) => {
  localStorage.setItem('sftp-columns', JSON.stringify(val))
}, { deep: true })

/** Loads the current directory. Returns false if the listing failed. */
async function loadFiles() {
  if (!props.sftpSessionId) return false
  loading.value = true
  try {
    const result = await store.sftpList(props.sftpSessionId, currentPath.value)
    files.value = result || []
    listError.value = ''
    return true
  } catch (e) {
    // Previously swallowed, so a dead session or an unreadable directory
    // looked like a panel that had simply stopped responding.
    console.error('sftp_list failed:', e)
    listError.value = String(e)
    return false
  } finally {
    loading.value = false
  }
}

/**
 * Move to `path`, reverting if the listing fails so the breadcrumb never
 * describes a directory other than the one whose rows are on screen.
 */
async function navigateTo(path) {
  const previous = currentPath.value
  clearSelection()
  filterQuery.value = ''
  currentPath.value = path
  if (!(await loadFiles())) {
    currentPath.value = previous
  }
}

function goUp() {
  if (currentPath.value === '/') return
  const parts = currentPath.value.split('/').filter(Boolean)
  parts.pop()
  navigateTo(parts.length === 0 ? '/' : '/' + parts.join('/'))
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
  await withErrorToast('Download', 'Download failed: ', async () => {
    const savedPath = await store.sftpDownload(props.sftpSessionId, file.path)
    toast(`Downloaded to ${savedPath}`, 'success')
  })
}

async function onDownloadDir() {
  const file = contextMenu.value.file
  if (!file || !file.is_dir) return
  contextMenu.value.show = false
  const transferKey = beginFolderTransfer(file)
  try {
    const savedPath = await invoke('sftp_download_dir', { sftpSessionId: props.sftpSessionId, remotePath: file.path })
    finishFolderTransfer(transferKey, file, '(saved)')
    toast(`Downloaded folder to ${savedPath}`, 'success')
  } catch (e) {
    console.error('Download folder failed:', e)
    finishFolderTransfer(transferKey, file, '(failed)')
    toast('Download folder failed: ' + e, 'error')
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
    onConfirm: () => withErrorToast('Delete', 'Delete failed: ', async () => {
      if (isDir) {
        await store.sftpRmdir(props.sftpSessionId, file.path)
      } else {
        await store.sftpDelete(props.sftpSessionId, file.path)
      }
      await loadFiles()
      toast(isDir ? `Deleted folder "${file.name}"` : `Deleted "${file.name}"`, 'success')
    }),
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
      await withErrorToast('mkdir', 'Failed to create folder: ', async () => {
        await store.sftpMkdir(props.sftpSessionId, fullPath)
        await loadFiles()
        toast(`Created folder "${name}"`, 'success')
      })
    },
  })
}

async function copyRemotePath() {
  const file = contextMenu.value.file
  if (!file) return
  contextMenu.value.show = false
  try {
    await writeText(file.path)
    toast('Path copied to clipboard', 'success')
  } catch (e) {
    console.warn('Copy path failed:', e)
    toast('Failed to copy path', 'error')
  }
}

async function onPreviewFile(file) {
  if (!file || file.is_dir) return
  // Close the editor to avoid overlapping panels, but let it prompt first
  // rather than discarding unsaved changes.
  if (editorRef.value && !(await editorRef.value.hide())) return
  previewModal.value = { show: true, fileName: file.name, filePath: file.path, fileSize: file.size || 0 }
}

async function previewEdit() {
  const filePath = previewModal.value.filePath
  const fileName = previewModal.value.fileName
  if (!filePath) return
  previewModal.value.show = false
  await onEditorOpen({ name: fileName, path: filePath })
}

async function previewDownload() {
  const filePath = previewModal.value.filePath
  if (!filePath) return
  previewModal.value.show = false
  await withErrorToast('Download', 'Download failed: ', async () => {
    const savedPath = await store.sftpDownload(props.sftpSessionId, filePath)
    toast(`Downloaded to ${savedPath}`, 'success')
  })
}

async function onEditorOpen(file) {
  if (!file || file.is_dir) return
  // Close preview if open to avoid overlapping panels
  if (previewModal.value.show) {
    previewModal.value.show = false
  }
  await editorRef.value?.open(file)
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
  await onEditorOpen(file)
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
      await withErrorToast('Rename', 'Rename failed: ', async () => {
        await store.sftpRename(props.sftpSessionId, file.path, newPath)
        await loadFiles()
        toast(`Renamed to "${newName}"`, 'success')
      })
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

  await openContextMenu(event, { file, multi: isMulti })
}

function onBulkDownloadFromMenu() {
  contextMenu.value.show = false
  bulkDownload()
}

function onBulkDeleteFromMenu() {
  contextMenu.value.show = false
  bulkDelete()
}
</script>
