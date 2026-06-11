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

    <!-- Editor floating panel -->
    <div
      v-if="editorModal.show"
      ref="editorModalRef"
      class="fixed z-50 bg-[#252526] border border-[#3c3c3c] rounded shadow-xl flex flex-col"
      :style="{ left: editorModal.x + 'px', top: editorModal.y + 'px', width: editorModal.width + 'px', height: editorModal.height + 'px', minWidth: '400px', minHeight: '250px' }"
    >
      <!-- Draggable title bar -->
      <div
        class="flex items-center justify-between px-3 py-2 border-b border-[#3c3c3c] shrink-0 select-none cursor-move bg-[#2d2d30]"
        @mousedown="startEditorDrag"
      >
        <span class="text-xs text-[#cccccc] truncate flex-1 mr-2">
          {{ editorModal.fileName }}
          <span v-if="editorModal.dirty" class="text-[#cca700] ml-1">●</span>
        </span>
        <div class="flex items-center gap-1.5 shrink-0">
          <button
            @click.stop="editorModal.wordWrap = !editorModal.wordWrap"
            class="text-[10px] px-1.5 py-0.5 rounded"
            :class="editorModal.wordWrap ? 'bg-[#007acc] text-white' : 'bg-[#3c3c3c] text-[#858585] hover:text-[#cccccc]'"
            title="Toggle word wrap"
          >↵ Wrap</button>
          <button
            @click.stop="onEditorSave"
            :disabled="editorModal.saving || !editorModal.dirty"
            class="text-[11px] px-2.5 py-1 rounded font-medium"
            :class="editorModal.dirty ? 'bg-[#89d185] hover:bg-[#73c16e] text-black' : 'bg-[#3c3c3c] text-[#858585] cursor-not-allowed'"
          >
            {{ editorModal.saving ? 'Saving...' : 'Save' }}
          </button>
          <button @click.stop="onEditorClose" class="text-[#858585] hover:text-[#cccccc] leading-none">×</button>
        </div>
      </div>
      <div class="flex-1 overflow-hidden flex">
        <div v-if="editorModal.loading" class="flex items-center justify-center h-full w-full text-[#858585] text-sm">
          Loading...
        </div>
        <template v-else>
          <!-- Line numbers -->
          <div
            ref="editorLineNumbersRef"
            class="shrink-0 bg-[#1e1e1e] text-[#6e6e6e] text-right select-none px-2 py-3 border-r border-[#3c3c3c] overflow-hidden"
            style="min-width: 2.5rem;"
          >
            <div v-for="n in editorLineCount" :key="n" class="text-[12px] leading-5 font-mono px-1">{{ n }}</div>
          </div>
          <!-- Textarea -->
          <textarea
            ref="editorTextareaRef"
            v-model="editorModal.content"
            @input="onEditorInput"
            @keydown="onEditorKeydown"
            @scroll="syncEditorScroll"
            class="flex-1 bg-[#1e1e1e] text-[#cccccc] text-[12px] font-mono p-3 resize-none focus:outline-none leading-5"
            :class="editorModal.wordWrap ? 'whitespace-pre-wrap break-all' : 'whitespace-pre'"
            spellcheck="false"
          ></textarea>
        </template>
      </div>
      <div class="px-3 py-1.5 border-t border-[#3c3c3c] text-[10px] text-[#6e6e6e] flex justify-between shrink-0">
        <span>{{ editorModal.content.length }} chars</span>
        <span v-if="editorModal.dirty" class="text-[#cca700]">Unsaved changes</span>
        <span v-else>Saved</span>
      </div>
      <!-- Resize handle -->
      <div
        class="absolute bottom-0 right-0 w-4 h-4 cursor-se-resize"
        style="background: linear-gradient(135deg, transparent 50%, #6e6e6e 50%);"
        @mousedown="startEditorResize"
        title="Resize"
      ></div>
    </div>

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
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from 'vue'
import { useConnectionStore } from '../stores/connection.js'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { openPath } from '@tauri-apps/plugin-opener'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { Folder, FileText, Home, ChevronRight } from 'lucide-vue-next'
import ConfirmDialog from './ConfirmDialog.vue'
import PromptDialog from './PromptDialog.vue'
import FilePreviewDialog from './FilePreviewDialog.vue'

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
  size: false,
  modified: false,
  perms: false,
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
  filePath: '',
  fileSize: 0,
})

const editorModal = ref({
  show: false,
  fileName: '',
  filePath: '',
  content: '',
  originalContent: '',
  loading: false,
  saving: false,
  dirty: false,
  wordWrap: false,
  width: 560,
  height: 400,
  x: typeof window !== 'undefined' ? window.innerWidth - 580 : 100,
  y: typeof window !== 'undefined' ? window.innerHeight - 420 : 100,
})

const editorModalRef = ref(null)
const editorLineNumbersRef = ref(null)
const editorTextareaRef = ref(null)
let editorResizeStart = null
let editorDragStart = null

const editorLineCount = computed(() => {
  if (!editorModal.value.content) return 1
  return editorModal.value.content.split('\n').length
})

function onEditorInput() {
  editorModal.value.dirty = true
}

function syncEditorScroll() {
  if (editorLineNumbersRef.value && editorTextareaRef.value) {
    editorLineNumbersRef.value.scrollTop = editorTextareaRef.value.scrollTop
  }
}

function startEditorDrag(e) {
  // Only drag on left mouse button, and not on buttons
  if (e.button !== 0 || e.target.closest('button')) return
  e.preventDefault()
  editorDragStart = { x: e.clientX, y: e.clientY, px: editorModal.value.x, py: editorModal.value.y }
  document.addEventListener('mousemove', onEditorDragMove)
  document.addEventListener('mouseup', onEditorDragUp)
}

function onEditorDragMove(e) {
  if (!editorDragStart) return
  const dx = e.clientX - editorDragStart.x
  const dy = e.clientY - editorDragStart.y
  editorModal.value.x = Math.max(0, editorDragStart.px + dx)
  editorModal.value.y = Math.max(0, editorDragStart.py + dy)
}

function onEditorDragUp() {
  editorDragStart = null
  document.removeEventListener('mousemove', onEditorDragMove)
  document.removeEventListener('mouseup', onEditorDragUp)
}

function startEditorResize(e) {
  e.preventDefault()
  editorResizeStart = { x: e.clientX, y: e.clientY, w: editorModal.value.width, h: editorModal.value.height }
  document.addEventListener('mousemove', onEditorResizeMove)
  document.addEventListener('mouseup', onEditorResizeUp)
}

function onEditorResizeMove(e) {
  if (!editorResizeStart) return
  const dx = e.clientX - editorResizeStart.x
  const dy = e.clientY - editorResizeStart.y
  editorModal.value.width = Math.max(400, editorResizeStart.w + dx)
  editorModal.value.height = Math.max(250, editorResizeStart.h + dy)
}

function onEditorResizeUp() {
  editorResizeStart = null
  document.removeEventListener('mousemove', onEditorResizeMove)
  document.removeEventListener('mouseup', onEditorResizeUp)
}

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
    const now = Date.now()
    const fileName = p.file.split('/').pop() || p.file
    const existing = transfers.value.find(t => t.file === p.file)
    if (existing) {
      const dt = (now - existing.lastUpdate) / 1000
      if (dt > 0) {
        const db = p.bytes_transferred - existing.bytes
        existing.speed = db / dt
      }
      existing.bytes = p.bytes_transferred
      existing.total = p.total_bytes
      existing.lastUpdate = now
      if (existing.bytes >= existing.total && !existing.done) {
        existing.done = true
        setTimeout(() => {
          transfers.value = transfers.value.filter(t => t.file !== p.file)
        }, 3000)
      }
    } else {
      const isDone = p.total_bytes === 0 || p.bytes_transferred >= p.total_bytes
      transfers.value.push({
        file: p.file,
        fileName,
        bytes: p.bytes_transferred,
        total: p.total_bytes,
        speed: 0,
        lastUpdate: now,
        done: isDone,
      })
      if (isDone) {
        setTimeout(() => {
          transfers.value = transfers.value.filter(t => t.file !== p.file)
        }, 3000)
      }
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
  const transferKey = `folder:${file.path}`
  transfers.value.push({
    file: transferKey,
    fileName: `📁 ${file.name}`,
    bytes: 0,
    total: 0,
    speed: 0,
    lastUpdate: Date.now(),
    done: false,
  })
  try {
    const savedPath = await invoke('sftp_download_dir', { sftpSessionId: props.sftpSessionId, remotePath: file.path })
    const t = transfers.value.find(x => x.file === transferKey)
    if (t) {
      t.done = true
      t.fileName = `📁 ${file.name} (saved)`
      setTimeout(() => {
        transfers.value = transfers.value.filter(x => x.file !== transferKey)
      }, 3000)
    }
    showToast(`Downloaded folder to ${savedPath}`, 'success')
  } catch (e) {
    console.error('Download folder failed:', e)
    const t = transfers.value.find(x => x.file === transferKey)
    if (t) {
      t.done = true
      t.fileName = `📁 ${file.name} (failed)`
      setTimeout(() => {
        transfers.value = transfers.value.filter(x => x.file !== transferKey)
      }, 3000)
    }
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
    await writeText(file.path)
    showToast('Path copied to clipboard', 'success')
  } catch (e) {
    console.warn('Copy path failed:', e)
    showToast('Failed to copy path', 'error')
  }
}

async function onPreviewFile(file) {
  if (!file || file.is_dir) return
  // Close editor if open to avoid overlapping panels
  if (editorModal.value.show) {
    editorModal.value.show = false
  }
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
  const fileName = previewModal.value.fileName
  if (!filePath) return
  previewModal.value.show = false
  try {
    const savedPath = await store.sftpDownload(props.sftpSessionId, filePath)
    showToast(`Downloaded to ${savedPath}`, 'success')
  } catch (e) {
    console.error('Download failed:', e)
    showToast('Download failed: ' + e, 'error')
  }
}

async function onEditorOpen(file) {
  if (!file || file.is_dir) return
  // Close preview if open to avoid overlapping panels
  if (previewModal.value.show) {
    previewModal.value.show = false
  }
  editorModal.value = {
    show: true,
    fileName: file.name,
    filePath: file.path,
    content: '',
    originalContent: '',
    loading: true,
    saving: false,
    dirty: false,
    wordWrap: false,
    width: editorModal.value.width,
    height: editorModal.value.height,
    x: editorModal.value.x,
    y: editorModal.value.y,
  }
  try {
    const content = await invoke('sftp_read_file', {
      sftpSessionId: props.sftpSessionId,
      remotePath: file.path,
    })
    editorModal.value.content = content
    editorModal.value.originalContent = content
  } catch (e) {
    console.error('Editor load failed:', e)
    showToast('Failed to load file: ' + e, 'error')
    editorModal.value.show = false
  } finally {
    editorModal.value.loading = false
  }
}

async function onEditorSave() {
  if (!editorModal.value.dirty || editorModal.value.saving) return
  editorModal.value.saving = true
  try {
    await invoke('sftp_write_file', {
      sftpSessionId: props.sftpSessionId,
      remotePath: editorModal.value.filePath,
      content: editorModal.value.content,
    })
    editorModal.value.originalContent = editorModal.value.content
    editorModal.value.dirty = false
    showToast(`Saved ${editorModal.value.fileName}`, 'success')
  } catch (e) {
    console.error('Save failed:', e)
    showToast('Save failed: ' + e, 'error')
  } finally {
    editorModal.value.saving = false
  }
}

function onEditorClose() {
  if (editorModal.value.dirty) {
    openConfirm({
      title: 'Unsaved Changes',
      message: `You have unsaved changes in "${editorModal.value.fileName}". Discard them?`,
      onConfirm: () => {
        editorModal.value.show = false
      },
    })
  } else {
    editorModal.value.show = false
  }
}

function onEditorKeydown(e) {
  if ((e.ctrlKey || e.metaKey) && e.key === 's') {
    e.preventDefault()
    onEditorSave()
  }
  if (e.key === 'Escape') {
    onEditorClose()
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

function formatSpeed(bytesPerSec) {
  if (bytesPerSec < 1024) return bytesPerSec.toFixed(0) + ' B/s'
  if (bytesPerSec < 1024 * 1024) return (bytesPerSec / 1024).toFixed(1) + ' KB/s'
  return (bytesPerSec / (1024 * 1024)).toFixed(1) + ' MB/s'
}
</script>
