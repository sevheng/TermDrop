<template>
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

  <ConfirmDialog
    :show="confirmDialog.show"
    :title="confirmDialog.title"
    :message="confirmDialog.message"
    :danger="confirmDialog.danger"
    confirm-text="Delete"
    @confirm="confirmDialog.onConfirm"
    @cancel="confirmDialog.onCancel"
  />
</template>

<script setup>
import { ref, computed } from 'vue'
import ConfirmDialog from './ConfirmDialog.vue'
import { invoke } from '../utils/invoke.js'
import { toast } from '../utils/toast.js'
import { useConfirmDialog } from '../composables/useConfirmDialog.js'

/**
 * Draggable, resizable floating text editor for a remote file. The parent
 * calls open(file) and hide(); position and size persist across files.
 */
const props = defineProps({
  sftpSessionId: {
    type: String,
    required: true,
  },
})

const { confirmDialog, openConfirm } = useConfirmDialog()

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

// Size and mtime as of the last load or save, so a save can tell whether
// someone else changed the file in the meantime.
let loadedStat = null

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


/**
 * Resolves true when the current buffer may be dropped: either it is clean,
 * or the user confirmed discarding it.
 */
function confirmDiscard() {
  if (!editorModal.value.dirty) return Promise.resolve(true)
  return new Promise((resolve) => {
    openConfirm({
      title: 'Unsaved Changes',
      message: `You have unsaved changes in "${editorModal.value.fileName}". Discard them?`,
      danger: true,
      onConfirm: () => resolve(true),
      onCancel: () => resolve(false),
    })
  })
}

async function readStat() {
  try {
    return await invoke('sftp_stat_file', {
      sftpSessionId: props.sftpSessionId,
      remotePath: editorModal.value.filePath,
    })
  } catch (e) {
    // Not being able to stat is not a reason to block a save.
    console.warn('stat failed:', e)
    return null
  }
}

async function open(file) {
  if (!file || file.is_dir) return
  // Opening a second file used to replace the buffer outright.
  if (!(await confirmDiscard())) return
  loadedStat = null
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
    loadedStat = await readStat()
  } catch (e) {
    console.error('Editor load failed:', e)
    toast('Failed to load file: ' + e, 'error')
    editorModal.value.show = false
  } finally {
    editorModal.value.loading = false
  }
}

async function onEditorSave() {
  if (!editorModal.value.dirty || editorModal.value.saving) return

  // Refuse to silently clobber a file that changed under us.
  const current = await readStat()
  if (
    loadedStat &&
    current &&
    (current.modified !== loadedStat.modified || current.size !== loadedStat.size)
  ) {
    const overwrite = await new Promise((resolve) => {
      openConfirm({
        title: 'File Changed on Server',
        message: `"${editorModal.value.fileName}" was modified on the server after you opened it. Overwrite those changes?`,
        danger: true,
        onConfirm: () => resolve(true),
        onCancel: () => resolve(false),
      })
    })
    if (!overwrite) return
  }

  editorModal.value.saving = true
  try {
    await invoke('sftp_write_file', {
      sftpSessionId: props.sftpSessionId,
      remotePath: editorModal.value.filePath,
      content: editorModal.value.content,
    })
    editorModal.value.originalContent = editorModal.value.content
    editorModal.value.dirty = false
    loadedStat = await readStat()
    toast(`Saved ${editorModal.value.fileName}`, 'success')
  } catch (e) {
    console.error('Save failed:', e)
    toast('Save failed: ' + e, 'error')
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

/**
 * Hide the editor, prompting first if there are unsaved changes. Returns
 * false if the user chose to keep editing, so a caller that was about to
 * open something on top can stand down.
 */
async function hide() {
  if (!(await confirmDiscard())) return false
  editorModal.value.show = false
  return true
}

const isOpen = computed(() => editorModal.value.show)

defineExpose({ open, hide, isOpen })
</script>
