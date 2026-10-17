#!/usr/bin/env node
// Regenerate the README's screenshots, demo GIF and social preview card.
// Usage:
//   npm run screenshots                       everything
//   npm run screenshots -- --only screens     just the screenshots (or gif, card)
//   npm run screenshots -- redis mongodb      just these screenshot scenes
//   npm run screenshots -- --skip-build       reuse the current dist/
//
// The app's built frontend runs in headless Chrome against demo-backend.js, a
// stand-in for the Tauri backend with fictional hosts and data, so the images
// show the real UI and nothing real. Output goes to .github/assets/.
//
// Needs Chrome or Chromium (or CHROME_PATH), ImageMagick and pngquant, plus
// ffmpeg for the GIF.

import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { serveApp, launchBrowser, sleep } from './lib.mjs'
import { captureScenes, SCENES } from './scenes.mjs'
import { recordDemo } from './demo-gif.mjs'

const ROOT = path.resolve(import.meta.dirname, '../..')
const DIST = path.join(ROOT, 'dist')
const ASSETS = path.join(ROOT, '.github/assets')
const SHOTS = path.join(ASSETS, 'screenshots')
const HERE = import.meta.dirname

const args = process.argv.slice(2)
const flag = name => args.includes(name)
const onlyIdx = args.indexOf('--only')
const parts = onlyIdx >= 0 ? args[onlyIdx + 1].split(',') : ['screens', 'gif', 'card']
const scenes = args.filter((a, i) => !a.startsWith('--') && !(onlyIdx >= 0 && i === onlyIdx + 1))
for (const s of scenes) if (!SCENES[s]) throw new Error(`unknown scene "${s}"; have: ${Object.keys(SCENES).join(', ')}`)

function has(cmd, versionFlag = '-version') {
  try {
    execFileSync(cmd, [versionFlag], { stdio: 'ignore' })
    return true
  } catch {
    return false
  }
}
const magick = has('magick') ? 'magick' : has('convert') ? 'convert' : null
const missing = [
  !magick && 'ImageMagick',
  !has('pngquant', '--version') && 'pngquant',
  parts.includes('gif') && !has('ffmpeg') && 'ffmpeg',
].filter(Boolean)
if (missing.length) {
  console.error(`Missing: ${missing.join(', ')}. Install them and run again.`)
  process.exit(1)
}

const run = (cmd, cmdArgs) => execFileSync(cmd, cmdArgs, { stdio: 'inherit', cwd: ROOT, shell: process.platform === 'win32' })
const im = (...a) => execFileSync(magick, a)
const quantize = (from, to) => execFileSync('pngquant', ['--quality=80-95', '--speed', '1', '--strip', '--force', '--output', to, from])

if (!flag('--skip-build')) run('npm', ['run', 'build'])
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'termdrop-shots-'))
const app = serveApp(DIST)
const browser = await launchBrowser()
let failed = []

try {
  if (parts.includes('screens')) {
    console.log('Screenshots')
    const raw = path.join(tmp, 'raw')
    fs.mkdirSync(raw)
    fs.mkdirSync(SHOTS, { recursive: true })
    failed = await captureScenes(browser, app.url(), raw, scenes)
    for (const file of fs.readdirSync(raw)) {
      const name = path.basename(file, '.png')
      const from = path.join(raw, file)
      const staged = path.join(tmp, file)
      if (name === 'add-host-light') {
        // Transparent outside the dialog's rounded corners, so the crop sits
        // cleanly on GitHub's light and dark pages alike.
        const [w, h] = im(from, '-format', '%w %h', 'info:').toString().split(' ').map(Number)
        im(from, '(', '-size', `${w}x${h}`, 'xc:none', '-fill', 'white', '-draw', `roundrectangle 0,0,${w - 1},${h - 1},16,16`, ')',
          '-alpha', 'set', '-compose', 'DstIn', '-composite', staged)
      } else if (name === 'local-shells') {
        fs.copyFileSync(from, staged)
      } else {
        im(from, '-resize', '1920x', '-strip', staged)
      }
      quantize(staged, path.join(SHOTS, file))
    }
  }

  if (parts.includes('gif')) {
    console.log('Demo GIF')
    const frames = path.join(tmp, 'frames')
    fs.mkdirSync(frames)
    const list = await recordDemo(browser, app.url(), frames)
    execFileSync('ffmpeg', ['-v', 'error', '-y', '-f', 'concat', '-safe', '0', '-i', list,
      '-vf', 'fps=12,scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=160:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle',
      '-loop', '0', path.join(ASSETS, 'demo.gif')])
    console.log('  ✓ demo.gif')
  }

  if (parts.includes('card')) {
    console.log('Social preview')
    const card = path.join(tmp, 'card')
    fs.mkdirSync(card)
    const fonts = path.join(ROOT, 'node_modules/@fontsource/ibm-plex-sans/files')
    for (const w of [400, 600]) fs.copyFileSync(path.join(fonts, `ibm-plex-sans-latin-${w}-normal.woff2`), path.join(card, `ibm-plex-sans-latin-${w}-normal.woff2`))
    fs.copyFileSync(path.join(ROOT, 'src-tauri/icons/128x128@2x.png'), path.join(card, 'icon.png'))
    fs.copyFileSync(path.join(SHOTS, 'terminal-dark.png'), path.join(card, 'shot.png'))
    fs.copyFileSync(path.join(HERE, 'social-card.html'), path.join(card, 'index.html'))
    const cardServer = serveApp(card, { inject: false })
    const page = await browser.newPage({ viewport: { width: 1280, height: 640 }, deviceScaleFactor: 1 })
    await page.goto(cardServer.url(), { waitUntil: 'networkidle' })
    await page.evaluate(() => document.fonts.ready)
    await sleep(200)
    await page.screenshot({ path: path.join(card, 'out.png') })
    cardServer.close()
    quantize(path.join(card, 'out.png'), path.join(ASSETS, 'social-preview.png'))
    console.log('  ✓ social-preview.png')
  }
} finally {
  await browser.close()
  app.close()
  fs.rmSync(tmp, { recursive: true, force: true })
}

if (failed.length) {
  console.error(`\n${failed.length} scene(s) failed: ${failed.join(', ')}`)
  process.exit(1)
}
console.log(`\nWritten to ${path.relative(ROOT, ASSETS)}/`)
