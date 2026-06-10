import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import './assets/main.css'
import { invoke } from '@tauri-apps/api/core'

const app = createApp(App)

// Global Vue error handler
app.config.errorHandler = (err, vm, info) => {
  const msg = `[Vue Error] ${info}: ${err?.message || err}`
  console.error(msg, err)
  window.dispatchEvent(new CustomEvent('app-toast', { detail: { message: msg, type: 'error' } }))
}

// Catch unhandled promise rejections
window.onunhandledrejection = (event) => {
  const msg = `[Unhandled Promise] ${event.reason?.message || event.reason}`
  console.error(msg, event.reason)
  window.dispatchEvent(new CustomEvent('app-toast', { detail: { message: msg, type: 'error' } }))
}

app.use(createPinia())
app.mount('#app')
