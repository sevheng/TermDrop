// Shared plumbing for the README screenshot tooling: serve the built app with
// the demo backend injected, find a Chrome to drive, and click by label.

import { chromium } from 'playwright-core'
import fs from 'node:fs'
import http from 'node:http'
import path from 'node:path'

const DEMO_BACKEND = fs.readFileSync(new URL('./demo-backend.js', import.meta.url), 'utf8')

const TYPES = {
  '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml',
  '.png': 'image/png', '.woff2': 'font/woff2', '.woff': 'font/woff',
}

/**
 * Serve `dist` on a free port, with the demo backend as the page's first
 * script unless `inject` is false.
 */
export function serveApp(dist, { inject = true } = {}) {
  const server = http.createServer((req, res) => {
    const { pathname } = new URL(req.url, 'http://x')
    const file = path.join(dist, pathname === '/' ? 'index.html' : decodeURIComponent(pathname))
    if (!file.startsWith(dist) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
      res.writeHead(404)
      return res.end()
    }
    let body = fs.readFileSync(file)
    if (inject && file.endsWith('index.html')) {
      body = body.toString().replace('<head>', `<head><script>${DEMO_BACKEND}</script>`)
    }
    res.writeHead(200, { 'content-type': TYPES[path.extname(file)] || 'application/octet-stream' })
    res.end(body)
  }).listen(0, '127.0.0.1')
  return {
    url: () => `http://127.0.0.1:${server.address().port}/`,
    close: () => server.close(),
  }
}

const CHROME_CANDIDATES = [
  '/usr/bin/google-chrome',
  '/usr/bin/google-chrome-stable',
  '/usr/bin/chromium',
  '/usr/bin/chromium-browser',
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  '/Applications/Chromium.app/Contents/MacOS/Chromium',
  'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
  'C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe',
]

export function findChrome() {
  const found = [process.env.CHROME_PATH, ...CHROME_CANDIDATES].find(p => p && fs.existsSync(p))
  if (!found) throw new Error('No Chrome or Chromium found; set CHROME_PATH to its executable.')
  return found
}

/**
 * WebGL is off on purpose: headless Chrome's software GL draws the xterm WebGL
 * renderer at double size under deviceScaleFactor 2. With it off the terminal
 * uses xterm's DOM renderer, which is what the app falls back to anyway.
 */
export function launchBrowser() {
  return chromium.launch({
    executablePath: findChrome(),
    headless: true,
    args: ['--no-sandbox', '--disable-webgl', '--disable-3d-apis', '--hide-scrollbars', '--font-render-hinting=none'],
  })
}

export const sleep = ms => new Promise(r => setTimeout(r, ms))

/** Open the app in `theme` and wait until it has drawn with its own fonts. */
export async function openApp(browser, base, { theme = 'dark', width = 1440, height = 900, scale = 2 } = {}) {
  // A page from browser.newPage has its own context, closed with the page.
  const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: scale })
  page.on('pageerror', e => console.error('  pageerror:', e.message))
  await page.goto(`${base}?theme=${theme}`, { waitUntil: 'networkidle' })
  await page.evaluate(() => document.fonts.ready)
  await sleep(600)
  return page
}

/** Centre of the first element whose own text is exactly `text`. */
export async function findText(page, text) {
  const box = await page.evaluate(text => {
    const el = [...document.querySelectorAll('*')].find(e =>
      [...e.childNodes].some(n => n.nodeType === 3 && n.textContent.trim() === text))
    if (!el) return null
    el.scrollIntoView({ block: 'center' })
    const r = el.getBoundingClientRect()
    return { x: r.x + r.width / 2, y: r.y + r.height / 2 }
  }, text)
  if (!box) throw new Error(`no element with text "${text}"`)
  return box
}

/**
 * Click like a person: a real pointer event at the element. `glide` moves the
 * pointer there first, so a recording shows what was clicked.
 */
export async function clickText(page, text, { glide = false } = {}) {
  const { x, y } = await findText(page, text)
  if (glide) await page.mouse.move(x, y, { steps: 6 })
  await page.mouse.click(x, y)
  await sleep(500)
}
