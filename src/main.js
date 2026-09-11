import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import './assets/main.css'
import { toast } from './utils/toast.js'
import { dismissSplash, logStartupPhase } from './utils/splash.js'
import { useConnectionStore } from './stores/connection.js'

performance.mark('td-bundle')

const app = createApp(App)

// Global Vue error handler
app.config.errorHandler = (err, vm, info) => {
  const msg = `[Vue Error] ${info}: ${err?.message || err}`
  console.error(msg, err)
  toast(msg, 'error')
}

// Catch unhandled promise rejections
window.onunhandledrejection = (event) => {
  const msg = `[Unhandled Promise] ${event.reason?.message || event.reason}`
  console.error(msg, event.reason)
  toast(msg, 'error')
}

app.use(createPinia())
app.mount('#app')
performance.mark('td-mounted')

// `mount` is synchronous and child onMounted hooks run inside it, so both
// MainWindow's and HostSidebar's calls to bootstrap() have already happened by
// now -- this one re-uses the memoised promise and costs no extra IPC.
dismissSplash(useConnectionStore().bootstrap())

// Attributes startup time to its phases, so a regression can be pinned to one
// rather than guessed at. `td-ready` is marked later, from splash.js, which
// reports that leg itself.
logStartupPhase('td-bundle')
logStartupPhase('td-mounted')
