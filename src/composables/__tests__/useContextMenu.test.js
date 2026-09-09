import { describe, it, expect } from 'vitest'
import { clampToViewport, useContextMenu } from '../useContextMenu.js'

const rect = { width: 100, height: 50 }

describe('clampToViewport', () => {
  it('leaves a menu that fits untouched, even inside the edge band', () => {
    expect(clampToViewport(300, 200, rect, 800, 600)).toEqual({ x: 300, y: 200 })
    expect(clampToViewport(700, 550, rect, 800, 600)).toEqual({ x: 700, y: 550 })
  })

  it('shifts an overflowing menu 8px inside the edge', () => {
    expect(clampToViewport(750, 580, rect, 800, 600)).toEqual({ x: 692, y: 542 })
  })

  it('never goes above 8px from the top-left', () => {
    expect(clampToViewport(2, 3, rect, 800, 600)).toEqual({ x: 8, y: 8 })
    expect(clampToViewport(0, 0, { width: 900, height: 700 }, 800, 600)).toEqual({ x: 8, y: 8 })
  })
})

describe('useContextMenu', () => {
  it('opens at the cursor with extra fields and closes', async () => {
    const menuEl = { value: null }
    const { contextMenu, openContextMenu, closeContextMenu } = useContextMenu(menuEl, { file: null })
    expect(contextMenu.value).toEqual({ show: false, x: 0, y: 0, file: null })
    await openContextMenu({ clientX: 10, clientY: 20 }, { file: 'f' })
    expect(contextMenu.value).toEqual({ show: true, x: 10, y: 20, file: 'f' })
    closeContextMenu()
    expect(contextMenu.value.show).toBe(false)
  })
})
