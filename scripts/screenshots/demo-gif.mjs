// Records the README demo: one frame per state, each with the time it stays on
// screen, written as an ffmpeg concat list that run.mjs encodes to a GIF.

import fs from 'node:fs'
import path from 'node:path'
import { openApp, clickText, sleep } from './lib.mjs'

export async function recordDemo(browser, base, outDir) {
  const page = await openApp(browser, base, { width: 1280, height: 800, scale: 1 })
  await page.mouse.move(1275, 795)

  const frames = []
  const frame = async hold => {
    const file = `f${String(frames.length).padStart(4, '0')}.png`
    await page.screenshot({ path: path.join(outDir, file) })
    frames.push({ file, hold })
  }
  // Capture continuously for `ms`; each frame is shown for the time it covered.
  const record = async ms => {
    const end = Date.now() + ms
    let last = Date.now()
    while (Date.now() < end) {
      await frame(0)
      const now = Date.now()
      frames.at(-1).hold = (now - last) / 1000
      last = now
    }
  }
  const click = text => clickText(page, text, { glide: true })

  await frame(1.6) // the host list
  await click('api-prod-01')
  await record(2600) // the tab opens and the session streams in
  await frame(2.2)
  await click('Docker'); await sleep(200); await frame(1.8)
  await click('Security'); await sleep(600); await frame(2.4)
  await click('Session cache'); await sleep(800)
  await click('user:1001'); await sleep(400); await frame(2.4)
  await click('Orders DB'); await sleep(600)
  await click('shop'); await sleep(100)
  await click('orders'); await sleep(700); await frame(2.6)

  const button = await (await page.$('[aria-label="New local terminal (Ctrl+Shift+T)"]')).boundingBox()
  const at = { x: button.x + button.width / 2, y: button.y + button.height / 2 }
  await page.mouse.move(at.x, at.y, { steps: 6 })
  await page.mouse.click(at.x, at.y)
  await sleep(700)
  await frame(2.4)
  await page.close()

  // The concat demuxer ignores the last entry's duration unless that file is
  // listed once more after it.
  const list = frames
    .map(f => `file '${f.file}'\nduration ${Math.max(f.hold, 0.04).toFixed(3)}`)
    .join('\n') + `\nfile '${frames.at(-1).file}'\n`
  fs.writeFileSync(path.join(outDir, 'list.txt'), list)
  return path.join(outDir, 'list.txt')
}
