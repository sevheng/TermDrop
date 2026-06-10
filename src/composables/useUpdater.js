import { check } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'

export async function checkForUpdates() {
  try {
    const update = await check()
    if (update) {
      return {
        available: true,
        version: update.version,
        notes: update.body,
        date: update.date,
        downloadAndInstall: async (onProgress) => {
          let downloaded = 0
          let contentLength = 0
          await update.downloadAndInstall((event) => {
            switch (event.event) {
              case 'Started':
                contentLength = event.data.contentLength
                break
              case 'Progress':
                downloaded += event.data.chunkLength
                if (onProgress && contentLength > 0) {
                  onProgress(Math.round((downloaded / contentLength) * 100))
                }
                break
              case 'Finished':
                if (onProgress) onProgress(100)
                break
            }
          })
          await relaunch()
        }
      }
    }
    return { available: false }
  } catch (err) {
    console.error('Update check failed:', err)
    return { available: false, error: String(err) }
  }
}
