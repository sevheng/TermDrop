<template>
  <!--
    A changed or revoked key. The connection is already torn down and no
    credential was sent before this renders.

    Its buttons are Close and Copy details. That is the complete list, and it
    is the point: the moment "connect anyway" is reachable from the warning,
    the warning is decoration, and every user who has met this in another
    client has learned that clicking through works. The remedy is deliberately
    somewhere else -- remove the entry, then verify the new key out of band.
  -->
  <ModalShell
    @close="$emit('close')" :show="show" dim="bg-black/70" z="z-50" panel-class="p-6 w-[34rem] shadow-xl">
      <div class="flex items-start gap-3 mb-4">
        <ShieldAlert :size="20" class="text-bad shrink-0 mt-0.5" />
        <div>
          <h3 class="text-lg font-semibold text-bad">Host key verification failed</h3>
          <p class="text-sm text-ink-2 mt-1">
            TermDrop did not connect, and your credentials were not sent.
          </p>
        </div>
      </div>

      <pre
        class="rounded border border-bad-line bg-bad-bg p-3 mb-5 text-xs text-ink font-mono whitespace-pre-wrap break-words max-h-64 overflow-y-auto"
      >{{ info.message }}</pre>

      <div class="flex justify-end gap-2">
        <button
          @click="copyDetails"
          class="px-4 py-2 text-sm text-ink-2 hover:text-ink rounded hover:bg-raised transition-colors"
        >
          Copy details
        </button>
        <button
          ref="closeEl"
          @click="$emit('close')"
          class="px-4 py-2 text-sm text-white rounded bg-bad-solid hover:opacity-90 transition-colors"
        >
          Close
        </button>
      </div>
  </ModalShell>
</template>

<script setup>
import { ref, watch, nextTick } from 'vue'
import { ShieldAlert } from 'lucide-vue-next'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { toast } from '../utils/toast.js'
import ModalShell from './ModalShell.vue'

const props = defineProps({
  show: Boolean,
  info: { type: Object, default: () => ({}) },
})
defineEmits(['close'])

const closeEl = ref(null)

async function copyDetails() {
  try {
    await writeText(props.info.message ?? '')
    toast('Copied', 'success')
  } catch {
    toast('Could not copy to the clipboard', 'error')
  }
}

watch(
  () => props.show,
  async shown => {
    if (!shown) return
    await nextTick()
    closeEl.value?.focus()
  },
)
</script>
