import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import './assets/main.css'
import { toast } from './utils/toast.js'

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
