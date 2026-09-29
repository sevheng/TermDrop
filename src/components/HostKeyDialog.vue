<template>
  <!-- First contact. An ordinary decision, so an ordinary dialog. -->
  <ModalShell
    @close="respond(false)" :show="show" dim="bg-black/60" z="z-50" panel-class="p-6 w-[30rem] shadow-xl">
      <h3 class="text-lg font-semibold text-ink mb-2">Trust this host?</h3>
      <p class="text-sm text-ink-2 mb-4">
        TermDrop has not connected to
        <span class="font-mono text-ink">{{ info.host }}:{{ info.port }}</span>
        before. Check the fingerprint below against the server before trusting it.
      </p>

      <div class="rounded border border-line bg-input p-3 mb-4 space-y-2">
        <div class="flex items-center justify-between gap-2">
          <span class="text-xs text-ink-2">Fingerprint</span>
          <span class="text-2xs text-ink-3">{{ info.keyType }}</span>
        </div>
        <div class="flex items-center gap-2">
          <code class="flex-1 text-xs text-ink font-mono break-all select-all">
            {{ info.fingerprint }}
          </code>
          <IconButton :icon="Copy" label="Copy fingerprint" @click="copy(info.fingerprint)" />
        </div>
      </div>

      <!-- The out-of-band check is the whole point of showing a fingerprint;
           giving the exact command makes it something people might do. -->
      <p class="text-xs text-ink-2 mb-1">Verify it from a machine you trust:</p>
      <div class="flex items-center gap-2 mb-5">
        <code class="flex-1 text-2xs text-ink-2 font-mono bg-input rounded px-2 py-1.5 break-all">
          {{ verifyCommand }}
        </code>
        <IconButton :icon="Copy" label="Copy command" @click="copy(verifyCommand)" />
      </div>

      <div class="flex justify-end gap-2">
        <button
          ref="cancelEl"
          @click="respond(false)"
          class="px-4 py-2 text-sm text-ink-2 hover:text-ink rounded hover:bg-raised transition-colors"
        >
          Cancel
        </button>
        <button
          @click="respond(true)"
          class="px-4 py-2 text-sm text-white rounded bg-accent-solid hover:bg-accent-solid-hover transition-colors"
        >
          Trust this host
        </button>
      </div>
  </ModalShell>
</template>

<script setup>
/**
 * Asking about a host key for the first time.
 *
 * Cancel is focused by default, and there is deliberately no "don't ask
 * again", no "trust every host in this group" and no global checkbox. The only
 * thing this dialog can do is trust exactly the key it is showing.
 */
import { ref, computed, watch, nextTick } from 'vue'
import { Copy } from 'lucide-vue-next'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { toast } from '../utils/toast.js'
import ModalShell from './ModalShell.vue'
import IconButton from './IconButton.vue'

const props = defineProps({
  show: Boolean,
  info: { type: Object, default: () => ({}) },
})
const emit = defineEmits(['respond'])

const cancelEl = ref(null)

const verifyCommand = computed(
  () =>
    `ssh-keyscan -p ${props.info.port ?? 22} -t ${props.info.keyType ?? ''} ` +
    `${props.info.host ?? ''} | ssh-keygen -lf -`,
)

function respond(accepted) {
  emit('respond', accepted)
}

async function copy(text) {
  try {
    await writeText(text)
    toast('Copied', 'success')
  } catch {
    toast('Could not copy to the clipboard', 'error')
  }
}

// Cancel takes focus, so Enter never trusts a host by accident.
watch(
  () => props.show,
  async shown => {
    if (!shown) return
    await nextTick()
    cancelEl.value?.focus()
  },
)
</script>
