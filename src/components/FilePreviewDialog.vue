<template>
  <div
    v-if="visible"
    class="fixed inset-0 bg-black/50 flex items-center justify-center z-50"
    @keydown.esc="$emit('close')"
    tabindex="-1"
  >
    <div
      ref="modalRef"
      class="bg-[#252526] border border-[#3c3c3c] rounded shadow-xl flex flex-col max-w-[90vw] max-h-[80vh]"
      :style="{ width: modalWidth + 'px', height: modalHeight + 'px', minWidth: '320px', minHeight: '200px' }"
    >
      <!-- Header -->
      <div class="flex items-center justify-between px-3 py-2 border-b border-[#3c3c3c] shrink-0 select-none">
        <span class="text-xs text-[#cccccc] truncate flex-1 mr-2">
          <template v-if="fileType === 'image'">🖼 Image:</template>
          <template v-else-if="fileType === 'binary'">📦 Binary:</template>
          <template v-else>📄 Preview:</template>
          {{ fileName }}
        </span>
        <div class="flex items-center gap-1.5 shrink-0">
          <!-- Search (text only) -->
          <template v-if="fileType === 'text'">
            <div class="flex items-center gap-1 bg-[#3c3c3c] rounded px-1.5 py-0.5">
              <Search :size="10" class="text-[#858585]" />
              <input
                v-model="searchQuery"
                type="text"
                placeholder="Find..."
                class="bg-transparent text-[10px] text-[#cccccc] w-24 focus:outline-none placeholder-[#6e6e6e]"
                @keydown.esc="searchQuery = ''"
              />
              <span v-if="searchQuery" class="text-[10px] text-[#858585]">
                {{ matchCount > 0 ? `${currentMatch + 1}/${matchCount}` : '0/0' }}
              </span>
              <button
                v-if="searchQuery && matchCount > 0"
                @click="prevMatch"
                class="text-[#858585] hover:text-[#cccccc] text-[10px] px-0.5"
                title="Previous"
              >▲</button>
              <button
                v-if="searchQuery && matchCount > 0"
                @click="nextMatch"
                class="text-[#858585] hover:text-[#cccccc] text-[10px] px-0.5"
                title="Next"
              >▼</button>
            </div>
          </template>
          <button @click="$emit('download')" class="text-[10px] text-[#858585] hover:text-[#cccccc] px-1.5 py-0.5 bg-[#3c3c3c] rounded" title="Download">
            ⬇
          </button>
          <button v-if="fileType === 'text'" @click="$emit('edit')" class="text-[10px] text-white bg-[#007acc] hover:bg-[#1177bb] px-2 py-0.5 rounded" title="Edit">
            Edit
          </button>
          <button @click="$emit('close')" class="text-[#858585] hover:text-[#cccccc] leading-none">×</button>
        </div>
      </div>

      <!-- Content -->
      <div class="flex-1 overflow-hidden relative">
        <!-- Loading -->
        <div v-if="loading" class="flex items-center justify-center h-full text-[#858585] text-sm">
          <Loader2 :size="16" class="animate-spin mr-2" /> Loading...
        </div>

        <!-- Error -->
        <div v-else-if="error" class="flex items-center justify-center h-full text-[#f44336] text-sm p-4 text-center">
          {{ error }}
        </div>

        <!-- Image -->
        <div v-else-if="fileType === 'image'" class="flex items-center justify-center h-full p-4">
          <img
            v-if="imageDataUrl"
            :src="imageDataUrl"
            :alt="fileName"
            class="max-w-full max-h-full object-contain rounded"
            draggable="false"
          />
        </div>

        <!-- Binary -->
        <div v-else-if="fileType === 'binary'" class="flex flex-col items-center justify-center h-full text-[#858585] p-4">
          <FileArchive :size="48" class="text-[#6e6e6e] mb-3" />
          <p class="text-sm text-[#cccccc] mb-1">Binary file</p>
          <p class="text-xs text-[#858585] mb-4">This file cannot be previewed.</p>
          <button
            @click="$emit('download')"
            class="text-xs bg-[#0e639c] hover:bg-[#1177bb] text-white px-3 py-1.5 rounded"
          >
            Download File
          </button>
        </div>

        <!-- Text with line numbers and search -->
        <div v-else class="flex h-full overflow-auto font-mono text-[11px] leading-5">
          <!-- Line numbers -->
          <div class="shrink-0 bg-[#1e1e1e] text-[#6e6e6e] text-right select-none px-2 py-3 border-r border-[#3c3c3c]" style="min-width: 3rem;">
            <div v-for="n in lineCount" :key="n" class="px-1">{{ n }}</div>
          </div>
          <!-- Content -->
          <div class="flex-1 overflow-auto py-3 px-3 whitespace-pre" ref="contentRef">
            <div
              v-for="(line, idx) in highlightedLines"
              :key="idx"
              class="px-1"
              :class="{ 'bg-[#cca700]/20': matchLines.has(idx) }"
            >
              <span v-html="line"></span>
            </div>
          </div>
        </div>
      </div>

      <!-- Resize handle -->
      <div
        class="absolute bottom-0 right-0 w-4 h-4 cursor-se-resize"
        style="background: linear-gradient(135deg, transparent 50%, #6e6e6e 50%);"
        @mousedown="startResize"
        title="Resize"
      ></div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, nextTick, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Loader2, Search, FileArchive } from 'lucide-vue-next'

const props = defineProps({
  visible: Boolean,
  fileName: { type: String, default: '' },
  filePath: { type: String, default: '' },
  fileSize: { type: Number, default: 0 },
  sftpSessionId: { type: String, default: '' },
})

const emit = defineEmits(['close', 'edit', 'download'])

const loading = ref(false)
const error = ref('')
const content = ref('')
const imageDataUrl = ref('')
const searchQuery = ref('')
const currentMatch = ref(0)
const contentRef = ref(null)
const modalRef = ref(null)

const modalWidth = ref(640)
const modalHeight = ref(480)
let resizeStart = null

const fileType = computed(() => detectFileType(props.fileName))

const lineCount = computed(() => {
  if (!content.value) return 0
  return content.value.split('\n').length
})

const lines = computed(() => {
  if (!content.value) return []
  return content.value.split('\n')
})

const matchLines = computed(() => {
  const set = new Set()
  if (!searchQuery.value.trim()) return set
  const q = searchQuery.value.toLowerCase()
  lines.value.forEach((line, idx) => {
    if (line.toLowerCase().includes(q)) set.add(idx)
  })
  return set
})

const matchCount = computed(() => matchLines.value.size)

const highlightedLines = computed(() => {
  const q = searchQuery.value.trim()
  if (!q) return lines.value.map(l => escapeHtml(l))
  const lowerQ = q.toLowerCase()
  return lines.value.map(line => {
    const lowerLine = line.toLowerCase()
    const parts = []
    let lastIndex = 0
    let idx = lowerLine.indexOf(lowerQ)
    while (idx !== -1) {
      parts.push(escapeHtml(line.slice(lastIndex, idx)))
      parts.push(`<mark class="bg-[#cca700]/40 text-[#cccccc]">${escapeHtml(line.slice(idx, idx + q.length))}</mark>`)
      lastIndex = idx + q.length
      idx = lowerLine.indexOf(lowerQ, lastIndex)
    }
    parts.push(escapeHtml(line.slice(lastIndex)))
    return parts.join('')
  })
})

function escapeHtml(text) {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
}

function detectFileType(fileName) {
  const ext = fileName.split('.').pop()?.toLowerCase() || ''
  const images = ['png', 'jpg', 'jpeg', 'gif', 'svg', 'webp', 'bmp', 'ico']
  const binaries = ['zip', 'tar', 'gz', 'bz2', 'xz', 'deb', 'rpm', 'exe', 'dll', 'so', 'dylib', 'bin', 'dat', 'pdf', 'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', '7z', 'rar']
  if (images.includes(ext)) return 'image'
  if (binaries.includes(ext)) return 'binary'
  return 'text'
}

function nextMatch() {
  if (matchCount.value === 0) return
  currentMatch.value = (currentMatch.value + 1) % matchCount.value
  scrollToMatch()
}

function prevMatch() {
  if (matchCount.value === 0) return
  currentMatch.value = (currentMatch.value - 1 + matchCount.value) % matchCount.value
  scrollToMatch()
}

function scrollToMatch() {
  nextTick(() => {
    if (!contentRef.value) return
    const matches = Array.from(matchLines.value)
    const lineIdx = matches[currentMatch.value]
    if (lineIdx == null) return
    const lineEl = contentRef.value.children[lineIdx]
    if (lineEl) {
      lineEl.scrollIntoView({ behavior: 'smooth', block: 'center' })
    }
  })
}

function startResize(e) {
  e.preventDefault()
  resizeStart = { x: e.clientX, y: e.clientY, w: modalWidth.value, h: modalHeight.value }
  document.addEventListener('mousemove', onResizeMove)
  document.addEventListener('mouseup', onResizeUp)
}

function onResizeMove(e) {
  if (!resizeStart) return
  const dx = e.clientX - resizeStart.x
  const dy = e.clientY - resizeStart.y
  modalWidth.value = Math.max(320, resizeStart.w + dx)
  modalHeight.value = Math.max(200, resizeStart.h + dy)
}

function onResizeUp() {
  resizeStart = null
  document.removeEventListener('mousemove', onResizeMove)
  document.removeEventListener('mouseup', onResizeUp)
}

async function loadContent() {
  if (!props.visible || !props.sftpSessionId || !props.filePath) return
  loading.value = true
  error.value = ''
  content.value = ''
  imageDataUrl.value = ''
  searchQuery.value = ''

  const type = fileType.value

  if (type === 'binary') {
    loading.value = false
    return
  }

  try {
    if (type === 'image') {
      const ext = props.fileName.split('.').pop()?.toLowerCase() || 'png'
      const mime = ext === 'svg' ? 'image/svg+xml' : ext === 'gif' ? 'image/gif' : ext === 'webp' ? 'image/webp' : ext === 'bmp' ? 'image/bmp' : ext === 'ico' ? 'image/x-icon' : 'image/png'
      const base64 = await invoke('sftp_read_file_base64', {
        sftpSessionId: props.sftpSessionId,
        remotePath: props.filePath,
      })
      imageDataUrl.value = `data:${mime};base64,${base64}`
    } else {
      const text = await invoke('sftp_read_file', {
        sftpSessionId: props.sftpSessionId,
        remotePath: props.filePath,
      })
      content.value = text
    }
  } catch (e) {
    error.value = String(e)
  } finally {
    loading.value = false
  }
}

function onKeydown(e) {
  if (e.key === 'Escape' && props.visible) {
    emit('close')
  }
}

onMounted(() => {
  document.addEventListener('keydown', onKeydown)
})

onUnmounted(() => {
  document.removeEventListener('keydown', onKeydown)
})

watch(() => [props.visible, props.filePath], () => {
  if (props.visible) {
    loadContent()
  }
}, { immediate: true })
</script>
