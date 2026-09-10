import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  // The vue plugin is what makes an SFC importable from a test. Without it
  // @vue/test-utils is installed but inert -- `mount()` fails to transform.
  plugins: [vue()],
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.js'],
  },
})
