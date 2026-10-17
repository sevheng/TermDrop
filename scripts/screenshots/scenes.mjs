// The README screenshots: each scene drives the real UI to a state and
// captures it at 1440x900, 2x. Names become the file names in
// .github/assets/screenshots/.

import path from 'node:path'
import { openApp, clickText, sleep } from './lib.mjs'

const SHELL_BUTTON = '[aria-label="New local terminal (Ctrl+Shift+T)"]'
const SHELL_MENU = '.min-w-\\[200px\\]'

/** Park the pointer where it hovers nothing, so no hover state is captured. */
const parkPointer = page => page.mouse.move(1435, 895)

/** The union of the selectors' boxes, padded and kept inside the viewport. */
async function around(page, selectors, pad) {
  const boxes = []
  for (const s of selectors) {
    const el = await page.$(s)
    if (el) boxes.push(await el.boundingBox())
  }
  if (!boxes.length) throw new Error('nothing to clip: ' + selectors.join(', '))
  const x = Math.max(0, Math.min(...boxes.map(b => b.x)) - pad)
  const y = Math.max(0, Math.min(...boxes.map(b => b.y)) - pad)
  const right = Math.min(1440, Math.max(...boxes.map(b => b.x + b.width)) + pad)
  const bottom = Math.min(900, Math.max(...boxes.map(b => b.y + b.height)) + pad)
  return { x, y, width: right - x, height: bottom - y }
}

/** An SSH tab on the first demo host, with its session fully streamed in. */
async function sshTab(browser, base, theme) {
  const page = await openApp(browser, base, { theme })
  await clickText(page, 'api-prod-01')
  await sleep(2500)
  await parkPointer(page)
  return page
}

/** Each scene returns the page to capture and, optionally, a clip box. */
export const SCENES = {
  'terminal-dark': async (b, base) => ({ page: await sshTab(b, base, 'dark') }),
  'terminal-light': async (b, base) => ({ page: await sshTab(b, base, 'light') }),
  'system-panel': async (b, base) => {
    const page = await sshTab(b, base, 'dark')
    await clickText(page, 'CPU:')
    await sleep(800)
    await parkPointer(page)
    return { page }
  },
  docker: async (b, base) => {
    const page = await sshTab(b, base, 'dark')
    await clickText(page, 'Docker')
    await sleep(900)
    await parkPointer(page)
    return { page }
  },
  security: async (b, base) => {
    const page = await sshTab(b, base, 'dark')
    await clickText(page, 'Security')
    await sleep(1200)
    await parkPointer(page)
    return { page }
  },
  tunnels: async (b, base) => {
    const page = await sshTab(b, base, 'dark')
    await clickText(page, 'Tunnels')
    await sleep(900)
    await parkPointer(page)
    return { page }
  },
  mongodb: async (b, base) => {
    const page = await openApp(b, base)
    await clickText(page, 'Orders DB')
    await sleep(1200)
    await clickText(page, 'shop')
    await sleep(700)
    await clickText(page, 'orders')
    await sleep(1500)
    await parkPointer(page)
    return { page }
  },
  redis: async (b, base) => {
    const page = await openApp(b, base)
    await clickText(page, 'Session cache')
    await sleep(1500)
    await clickText(page, 'user:1001')
    await sleep(1200)
    await parkPointer(page)
    return { page }
  },
  'local-shells': async (b, base) => {
    const page = await openApp(b, base)
    await page.waitForSelector(SHELL_BUTTON, { timeout: 10000 })
    await page.click(SHELL_BUTTON)
    await page.waitForSelector(SHELL_MENU, { timeout: 5000 })
    await sleep(500)
    await page.mouse.move(1000, 600)
    return { page, clip: await around(page, [SHELL_BUTTON, SHELL_MENU], 24) }
  },
  // Clipped tight to the dialog: any padding catches the empty-state text
  // behind the overlay. run.mjs rounds the corners to transparent.
  'add-host-light': async (b, base) => {
    const page = await openApp(b, base, { theme: 'light' })
    await page.evaluate(() => window.dispatchEvent(new CustomEvent('open-host-modal')))
    await sleep(800)
    await parkPointer(page)
    return { page, clip: await around(page, ['[role="dialog"]'], 0) }
  },
}

/** Capture the named scenes (all when `only` is empty) into `outDir`. */
export async function captureScenes(browser, base, outDir, only = []) {
  const failed = []
  for (const [name, scene] of Object.entries(SCENES)) {
    if (only.length && !only.includes(name)) continue
    try {
      const { page, clip } = await scene(browser, base)
      await sleep(400)
      await page.screenshot({ path: path.join(outDir, `${name}.png`), ...(clip ? { clip } : {}) })
      await page.close()
      console.log('  ✓', name)
    } catch (e) {
      failed.push(name)
      console.error('  ✗', name, e.message)
    }
  }
  return failed
}
