<template>
  <div class="w-80 h-full bg-gray-800 border-l border-gray-700 flex flex-col">
    <!-- Header -->
    <div class="p-3 border-b border-gray-700">
      <h3 class="text-sm font-semibold text-gray-200 mb-2">SFTP Browser</h3>
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
    <div class="px-3 py-2 border-b border-gray-700 flex gap-2">
      <button @click="goUp" class="text-xs bg-gray-700 hover:bg-gray-600 text-white px-2 py-1 rounded">↑ Up</button>
      <button @click="onUpload" class="text-xs bg-blue-600 hover:bg-blue-700 text-white px-2 py-1 rounded">Upload</button>
      <button @click="loadFiles" class="text-xs bg-gray-700 hover:bg-gray-600 text-white px-2 py-1 rounded">↻</button>
    </div>

    <!-- File list -->
    <div class="flex-1 overflow-y-auto">
      <div
        v-if="loading"
        class="flex items-center justify-center h-20 text-gray-500 text-sm"
      >
        Loading...
      </div>
      <div v-else-if="files.length === 0" class="flex items-center justify-center h-20 text-gray-500 text-sm">
        Empty directory
      </div>
      <div v-else>
        <div
          v-for="file in files"
          :key="file.path"
          class="flex items-center gap-2 px-3 py-1.5 hover:bg-gray-700 cursor-pointer text-sm"
          :class="file.is_dir ? 'text-blue-300' : 'text-gray-200'"
          @dblclick="file.is_dir && navigateTo(file.path)"
          @contextmenu.prevent="showContextMenu($event, file)"
        >
          <Folder v-if="file.is_dir" :size="14" class="shrink-0" />
          <FileText v-else :size="14" class="shrink-0 text-gray-400" />
          <span class="truncate flex-1 min-w-0">{{ file.name }}</span>
          <span v-if="!file.is_dir" class="text-xs text-gray-500 shrink-0">{{ formatSize(file.size) }}</span>
        </div>
      </div>
    </div>

    <!-- Context menu -->
    <div
      v-if="contextMenu.show"
      class="absolute bg-gray-700 border border-gray-600 rounded shadow-lg py-1 z-50"
      :style="{ top: contextMenu.y + 'px', left: contextMenu.x + 'px' }"
    >
      <button @click="onDownload" class="block w-full text-left px-4 py-1.5 text-sm text-white hover:bg-gray-600">Download</button>
      <button @click="onRename" class="block w-full text-left px-4 py-1.5 text-sm text-white hover:bg-gray-600">Rename</button>
      <button @click="onDelete" class="block w-full text-left px-4 py-1.5 text-sm text-red-400 hover:bg-gray-600">Delete</button>
    </div>

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
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { useConnectionStore } from '../stores/connection.js'
import { listen } from '@tauri-apps/api/event'
import { Folder, FileText } from 'lucide-vue-next'

const props = defineProps({
  sftpSessionId: {
    type: String,
    required: true,
  },
})

const store = useConnectionStore()
const currentPath = ref('/home')
const files = ref([])
const loading = ref(false)
const contextMenu = ref({ show: false, x: 0, y: 0, file: null })
const transfers = ref([])
let unlistenProgress = null

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

onMounted(async () => {
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
})

onUnmounted(() => {
  if (unlistenProgress) unlistenProgress()
})

watch(() => props.sftpSessionId, async () => {
  currentPath.value = '/home'
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
    console.log('Downloaded to:', savedPath)
  } catch (e) {
    console.error('Download failed:', e)
  }
}

async function onDelete() {
  const file = contextMenu.value.file
  if (!file) return
  contextMenu.value.show = false
  if (!confirm(`Delete ${file.name}?`)) return
  try {
    await store.sftpDelete(props.sftpSessionId, file.path)
    await loadFiles()
  } catch (e) {
    console.error('Delete failed:', e)
  }
}

async function onRename() {
  const file = contextMenu.value.file
  if (!file) return
  contextMenu.value.show = false
  const newName = prompt('New name:', file.name)
  if (!newName || newName === file.name) return
  const parent = file.path.substring(0, file.path.lastIndexOf('/')) || '/'
  const newPath = parent === '/' ? '/' + newName : parent + '/' + newName
  try {
    await store.sftpRename(props.sftpSessionId, file.path, newPath)
    await loadFiles()
  } catch (e) {
    console.error('Rename failed:', e)
  }
}

function showContextMenu(event, file) {
  contextMenu.value = {
    show: true,
    x: event.clientX,
    y: event.clientY,
    file,
  }
}

function formatSize(bytes) {
  if (bytes < 1024) return bytes + ' B'
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB'
  if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + ' MB'
  return (bytes / (1024 * 1024 * 1024)).toFixed(1) + ' GB'
}

// Close context menu on click outside
window.addEventListener('click', () => {
  contextMenu.value.show = false
})
</script>
