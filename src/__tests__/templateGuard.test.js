import { describe, it, expect } from 'vitest'
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'

/**
 * Template mistakes that compile, render, and silently do nothing.
 *
 * Both rules below cost a real debugging session: the page looked right, so
 * the search went everywhere except the binding.
 */
function walk(dir, out = []) {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry)
    if (statSync(full).isDirectory()) walk(full, out)
    else if (full.endsWith('.vue')) out.push(full)
  }
  return out
}

const files = walk('src')

function findLines(pattern) {
  const hits = []
  for (const file of files) {
    readFileSync(file, 'utf8')
      .split('\n')
      .forEach((line, i) => {
        if (pattern.test(line)) hits.push(`${file}:${i + 1}  ${line.trim()}`)
      })
  }
  return hits
}

describe('template guard', () => {
  it('finds the components to check', () => {
    // Guards the guard: a broken walk would make everything below vacuous.
    expect(files.length).toBeGreaterThan(20)
  })

  it('binds events one by one rather than spreading an object', () => {
    // `v-on="obj"` runs every key through `toHandlers`, which prefixes it:
    // `{ onPointerMove }` becomes a listener for `onpointermove`, an event
    // that does not exist. Nothing warns, nothing throws, and no handler ever
    // fires. It cost the VNC canvas its entire mouse and keyboard.
    //
    // The object form is only correct with bare event names (`pointermove`),
    // which is a distinction too easy to lose. Write `@pointermove="fn"`.
    const hits = findLines(/\sv-on="[^"]/)
    expect(hits, `use explicit @event bindings instead:\n${hits.join('\n')}`).toEqual([])
  })

  it('passes ModalShell its content as the default slot', () => {
    // ModalShell has one unnamed `<slot />` -- no `title` prop and no `footer`
    // slot. Content aimed at a slot that does not exist is dropped in silence,
    // which is how a dialog shipped with no heading and no Save button.
    const hits = findLines(/<ModalShell[^>]*\stitle=|#footer/)
    expect(
      hits,
      `ModalShell has no title prop and no footer slot:\n${hits.join('\n')}`,
    ).toEqual([])
  })
})
