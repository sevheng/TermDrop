import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [vue()],

  build: {
    // Vite 8 bundles with Rolldown, which has no object-form manualChunks.
    // The same two chunks are expressed as code-splitting groups.
    rolldownOptions: {
      output: {
        codeSplitting: {
          groups: [
            { name: "xterm", test: /[\\/]node_modules[\\/]@xterm[\\/]/ },
            {
              name: "vendor",
              test: /[\\/]node_modules[\\/](vue|@vue|pinia|lucide-vue-next)[\\/]/,
            },
          ],
        },
      },
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
