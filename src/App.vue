<template>
  <div v-if="fatalError" class="error-boundary">
    <div class="error-content">
      <h2>💥 Something went wrong</h2>
      <p>{{ fatalError }}</p>
      <button @click="reload">Reload App</button>
      <button @click="dismiss" class="secondary">Dismiss</button>
    </div>
  </div>
  <template v-else>
    <MainWindow @update-available="onUpdateAvailable" />
    <ToastContainer />
    <UpdateModal
      :show="updateModal.show"
      :version="updateModal.version"
      :notes="updateModal.notes"
      :downloadAndInstall="updateModal.downloadAndInstall"
      @close="updateModal.show = false"
    />
  </template>
</template>

<script setup>
import { ref, onErrorCaptured, onMounted } from 'vue'
import MainWindow from './views/MainWindow.vue'
import ToastContainer from './components/ToastContainer.vue'
import UpdateModal from './components/UpdateModal.vue'
import { checkForUpdates } from './composables/useUpdater.js'

const fatalError = ref(null)

const updateModal = ref({
  show: false,
  version: '',
  notes: '',
  downloadAndInstall: null,
})

onErrorCaptured((err, instance, info) => {
  fatalError.value = `${info}: ${err?.message || err}`
  console.error('[Error Boundary]', fatalError.value, err)
  return false // prevent propagation
})

onMounted(async () => {
  // Check for updates on startup
  try {
    const result = await checkForUpdates()
    if (result.available) {
      updateModal.value = {
        show: true,
        version: result.version,
        notes: result.notes,
        downloadAndInstall: result.downloadAndInstall,
      }
    }
  } catch (err) {
    console.error('Startup update check failed:', err)
  }
})

function reload() {
  window.location.reload()
}

function dismiss() {
  fatalError.value = null
}

function onUpdateAvailable(result) {
  updateModal.value = {
    show: true,
    version: result.version,
    notes: result.notes,
    downloadAndInstall: result.downloadAndInstall,
  }
}
</script>

<style scoped>
.error-boundary {
  position: fixed;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #1e1e1e;
  color: #e0e0e0;
  z-index: 9999;
}
.error-content {
  text-align: center;
  padding: 2rem;
  max-width: 480px;
}
.error-content h2 {
  margin-bottom: 1rem;
  color: #ff6b6b;
}
.error-content p {
  margin-bottom: 1.5rem;
  font-family: monospace;
  font-size: 0.85rem;
  opacity: 0.8;
  word-break: break-word;
}
.error-content button {
  padding: 0.5rem 1.5rem;
  margin: 0 0.5rem;
  border: none;
  border-radius: 4px;
  background: #4a9eff;
  color: white;
  cursor: pointer;
  font-size: 0.9rem;
}
.error-content button.secondary {
  background: transparent;
  border: 1px solid #555;
}
.error-content button:hover {
  opacity: 0.9;
}
</style>
