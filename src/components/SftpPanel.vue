<template>
  <div class="w-72 h-full bg-gray-800 border-l border-gray-700 flex flex-col relative">
    <!-- Header -->
    <div class="p-2 border-b border-gray-700">
      <h3 class="text-sm font-semibold text-gray-200 mb-1">SFTP Browser</h3>
      <div class="flex items-center gap-1 text-xs text-gray-400 overflow-x-auto whitespace-nowrap">
        <span
          v-for="(segment, index) in breadcrumbs"
          :key="index"
          class="cursor-pointer hover:text-white"
          @click="navigateTo(segment.path)"
        >
          {{ segment.name }}<span v-if="index < breadcrumbs.length - 1" class="mx-1">/</span>
        </span>
      </div>
    </div>

    <!-- Toolbar -->
    <div class="px-2 py-1.5 border-b border-gray-700 flex gap-1.5 flex-wrap">
      <button @click="goUp" class="text-xs bg-gray-700 hover:bg-gray-600 text-white px-1.5 py-0.5 rounded">↑ Up</button>
      <button @click="onUpload" class="text-xs bg-blue-600 hover:bg-blue-700 text-white px-1.5 py-0.5 rounded">Upload</button>
      <button @click="onMkdir" class="text-xs bg-gray-700 hover:bg-gray-600 text-white px-1.5 py-0.5 rounded">+ Folder</button>
      <button @click="loadFiles" class="text-xs bg-gray-700 hover:bg-gray-600 text-white px-1.5 py-0.5 rounded">↻</button>
    </div>

    <!-- Column headers -->
    <div class="px-2 py-0.5 border-b border-gray-700 flex items-center text-xs text-gray-400 select-none">
      <span class="flex-1 min-w-0 cursor-pointer hover:text-white" @click="setSort('name')">
        Name {{ sortIndicator('name') }}
      </span>
      <span class="w-12 shrink-0 text-right cursor-pointer hover:text-white" @click="setSort('size')">
        Size {{ sortIndicator('size') }}
      </span>
      <span class="w-14 shrink-0 text-right cursor-pointer hover:text-white ml-1.5" @click="setSort('modified')">
        Modified {{ sortIndicator('modified') }}
      </span>
      <span class="w-16 shrink-0 text-right ml-1.5">Perms</span>
    </div>

    <!-- File list -->
    <div class="flex-1 overflow-y-auto relative">
      <div
        v-if="loading"
        class="flex items-center justify-center h-20 text-gray-500 text-sm"
      >
        Loading...
      </div>
      <div v-else-if="sortedFiles.length === 0" class="flex items-center justify-center h-20 text-gray-500 text-sm">
        Empty directory
      </div>
      <div v-else>
        <div
          v-for="file in sortedFiles"
          :key="file.path"
          class="flex items-center px-2 py-0.5 hover:bg-gray-700 cursor-pointer text-sm"
          :class="file.is_dir ? 'text-blue-300' : 'text-gray-200'"
          @dblclick="file.is_dir && navigateTo(file.path)"
          @contextmenu.prevent="showContextMenu($event, file)"
        >
          <Folder v-if="file.is_dir" :size="14" class="shrink-0 mr-1.5" />
          <FileText v-else :size="14" class="shrink-0 mr-1.5 text-gray-400" />
          <span class="truncate flex-1 min-w-0">{{ file.name }}</span>
          <span class="w-12 shrink-0 text-right text-xs text-gray-500">{{ file.is_dir ? '-' : formatSize(file.size) }}</span>
          <span class="w-14 shrink-0 text-right text-xs text-gray-500 ml-1.5">{{ formatDate(file.modified) }}</span>
          <span class="w-16 shrink-0 text-right text-xs text-gray-500 ml-1.5 font-mono">{{ formatPermissions(file.permissions, file.is_dir) }}</span>
        </div>
      </div>
    </div>

    <!-- Context menu -->
    <div
      v-if="contextMenu.show"
      ref="contextMenuEl"
      class="fixed bg-gray-700 border border-gray-600 rounded shadow-lg py-1 z-50 min-w-[8rem]"
      :style="{ top: contextMenu.y + 'px', left: contextMenu.x + 'px' }"
    >
      <button @click="onDownload" class="block w-full text-left px-4 py-1.5 text-sm text-white hover:bg-gray-600">Download</button>
      <button @click="copyRemotePath" class="block w-full text-left px-4 py-1.5 text-sm text-white hover:bg-gray-600">Copy Path</button>
      <button @click="onRename" class="block w-full text-left px-4 py-1.5 text-sm text-white hover:bg-gray-600">Rename</button>
      <button @click="onDelete" class="block w-full text-left px-4 py-1.5 text-sm text-red-400 hover:bg-gray-600">Delete</button>
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

    <!-- Progress toasts -->
    <div v-if="transfers.length > 0" class="border-t border-gray-700 p-2 space-y-2">
      <div v-for="t in transfers" :key="t.file" class="text-xs">
        <div class="flex justify-between text-gray-300 mb-1">
          <span class="truncate">{{ t.file }}</span>
          <span>{{ Math.round((t.bytes / t.total) * 100) }}%</span>
        </div>
        <div class="h-1 bg-gray-700 rounded overflow-hidden">
          <div class="h-full bg-blue-500 transition-all" :style="{ width: (t.bytes / t.total) * 100 + '%' }"></div>
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
import { Folder, FileText } from 'lucide-vue-next'
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
}

onMounted(async () => {
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
})

watch(() => props.sftpSessionId, async () => {
  await resolveHomeDir()
  await loadFiles()
})

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
  currentPath.value = path
  loadFiles()
}

function goUp() {
  if (currentPath.value === '/') return
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
  contextMenu.value = {
    show: true,
    x: event.clientX,
    y: event.clientY,
    file,
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

function formatSize(bytes) {
  if (bytes < 1024) return bytes + ' B'
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB'
  if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + ' MB'
  return (bytes / (1024 * 1024 * 1024)).toFixed(1) + ' GB'
}
</script>
