<template>
  <div class="relative" :class="block ? 'w-full' : 'inline-block'">
    <button
      ref="triggerEl"
      type="button"
      role="combobox"
      :aria-expanded="open"
      :aria-controls="open ? listId : undefined"
      aria-haspopup="listbox"
      :disabled="disabled"
      :class="[
        'flex items-center gap-2 rounded bg-input text-ink border border-line',
        'hover:border-line focus:outline-none focus:ring-1 focus:ring-accent',
        'disabled:opacity-50 disabled:cursor-not-allowed transition-colors',
        block ? 'w-full justify-between' : '',
        size === 'xs' ? 'text-2xs px-2 py-0.5' : 'text-xs px-2 py-1.5',
      ]"
      @click="toggle"
      @keydown="onTriggerKey"
    >
      <span :class="['truncate', selected ? 'text-ink' : 'text-ink-3']">
        {{ selected ? selected.label : placeholder }}
      </span>
      <ChevronDown
        :size="size === 'xs' ? 11 : 13"
        class="shrink-0 text-ink-3 transition-transform"
        :class="open ? 'rotate-180' : ''"
      />
    </button>

    <!--
      Fixed, not absolute: every one of these lives inside a panel or modal
      with its own overflow, and an absolutely-positioned menu gets clipped by
      the first one of those it meets.
    -->
    <ul
      v-if="open"
      :id="listId"
      ref="listEl"
      role="listbox"
      tabindex="-1"
      :aria-activedescendant="active >= 0 ? `${listId}-${active}` : undefined"
      class="fixed z-[200] overflow-y-auto rounded border border-line bg-overlay shadow-xl py-1"
      :style="{ top: `${pos.top}px`, left: `${pos.left}px`, width: `${pos.width}px`, maxHeight: `${pos.maxHeight}px` }"
      @keydown="onListKey"
    >
      <li
        v-for="(opt, i) in options"
        :id="`${listId}-${i}`"
        :key="String(opt.value)"
        role="option"
        :aria-selected="opt.value === modelValue"
        :aria-disabled="opt.disabled || undefined"
        :class="[
          'flex items-center gap-2 px-2 cursor-pointer',
          size === 'xs' ? 'text-2xs py-1' : 'text-xs py-1.5',
          opt.disabled ? 'text-ink-3 cursor-not-allowed' : 'text-ink',
          i === active && !opt.disabled ? 'bg-raised' : '',
        ]"
        @mouseenter="opt.disabled || (active = i)"
        @click="opt.disabled || choose(i)"
      >
        <Check
          :size="12"
          class="shrink-0"
          :class="opt.value === modelValue ? 'text-accent' : 'opacity-0'"
        />
        <span class="truncate flex-1">{{ opt.label }}</span>
        <span v-if="opt.hint" class="shrink-0 text-ink-3 text-2xs">{{ opt.hint }}</span>
      </li>
      <li v-if="options.length === 0" class="px-2 py-1.5 text-xs text-ink-3 italic">
        Nothing to choose
      </li>
    </ul>
  </div>
</template>

<script setup>
/**
 * A select that the app draws itself.
 *
 * The native control is drawn by the platform: on WebKitGTK it ignores the
 * background it is given, and its dropdown is a separate native surface that
 * cannot be themed at all. That left it looking foreign in both themes and,
 * before `color-scheme` was declared, unreadable in one.
 *
 * Keyboard behaviour follows the WAI-ARIA combobox pattern, because replacing
 * a native control means taking on what it did for free: arrows and Home/End
 * move, Enter and Space choose, Escape closes without changing anything, and
 * typing jumps to a matching option.
 */
import { ref, computed, watch, onBeforeUnmount, nextTick } from 'vue'
import { ChevronDown, Check } from 'lucide-vue-next'
import {
  nextEnabledIndex,
  firstEnabledIndex,
  lastEnabledIndex,
  indexOfValue,
  typeaheadIndex,
  menuPlacement,
  TYPEAHEAD_MS,
} from '../utils/selectNav.js'

const props = defineProps({
  /** The chosen value. Compared by identity, so '' and 0 are real choices. */
  modelValue: { type: [String, Number, null], default: null },
  /** `[{ value, label, hint?, disabled? }]` */
  options: { type: Array, required: true },
  placeholder: { type: String, default: 'Select…' },
  size: { type: String, default: 'sm' },
  disabled: { type: Boolean, default: false },
  /** Fill the container, as a form field does. */
  block: { type: Boolean, default: false },
})

const emit = defineEmits(['update:modelValue', 'change'])

let seq = 0
const listId = `select-${(seq += 1)}-${Math.random().toString(36).slice(2, 7)}`

const open = ref(false)
const active = ref(-1)
const triggerEl = ref(null)
const listEl = ref(null)
const pos = ref({ top: 0, left: 0, width: 0, maxHeight: 320 })

const selected = computed(() => props.options.find(o => o.value === props.modelValue) || null)

/** Measure the trigger and decide where the list goes. */
async function place() {
  const el = triggerEl.value
  if (!el) return
  const rect = el.getBoundingClientRect()
  // Rough height first so the flip decision is made before paint, then
  // refined once the list has actually rendered.
  const rowH = props.size === 'xs' ? 22 : 28
  const guess = Math.min(props.options.length * rowH + 8, 320)
  const p = menuPlacement(rect, guess, window.innerHeight)
  pos.value = { top: p.top, left: Math.round(rect.left), width: Math.round(rect.width), maxHeight: p.maxHeight }

  await nextTick()
  const list = listEl.value
  if (!list) return
  const real = menuPlacement(rect, list.scrollHeight, window.innerHeight)
  pos.value = { ...pos.value, top: real.top, maxHeight: real.maxHeight }
}

/**
 * Reveal the chosen option. Only when opening.
 *
 * This used to live at the end of place(), which also runs on every scroll —
 * so scrolling the list scrolled it straight back to the selection, and the
 * menu could not be scrolled at all.
 */
function revealSelected() {
  // Optional call: scrollIntoView is absent in some environments, and
  // failing to reveal a row must never break opening the menu.
  listEl.value?.querySelector('[aria-selected="true"]')?.scrollIntoView?.({ block: 'nearest' })
}

async function openMenu() {
  if (props.disabled) return
  open.value = true
  const current = indexOfValue(props.options, props.modelValue)
  active.value = current >= 0 ? current : firstEnabledIndex(props.options)
  await place()
  revealSelected()
  listEl.value?.focus()
}

function closeMenu({ refocus = true } = {}) {
  open.value = false
  active.value = -1
  query = ''
  if (refocus) triggerEl.value?.focus()
}

function toggle() {
  open.value ? closeMenu() : openMenu()
}

function choose(i) {
  const opt = props.options[i]
  if (!opt || opt.disabled) return
  if (opt.value !== props.modelValue) {
    emit('update:modelValue', opt.value)
    // A separate event so callers can re-run a query only on a real change,
    // the way @change on the native control behaved.
    emit('change', opt.value)
  }
  closeMenu()
}

// --- keyboard ---------------------------------------------------------------

let query = ''
let queryTimer = null

function onTypeahead(key) {
  clearTimeout(queryTimer)
  query += key.toLowerCase()
  queryTimer = setTimeout(() => {
    query = ''
  }, TYPEAHEAD_MS)
  // Repeating one letter walks the options starting with it; anything longer
  // restarts the search from the current row.
  const from = query.length === 1 ? active.value : active.value - 1
  const i = typeaheadIndex(props.options, query, from)
  if (i >= 0) {
    active.value = i
    scrollActiveIntoView()
  }
}

function scrollActiveIntoView() {
  nextTick(() => {
    listEl.value?.querySelector(`#${CSS.escape(listId)}-${active.value}`)
      ?.scrollIntoView?.({ block: 'nearest' })
  })
}

function onTriggerKey(e) {
  if (open.value) return
  if (['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(e.key)) {
    e.preventDefault()
    openMenu()
  } else if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
    // Typing on a closed control jumps straight to a match, as the native
    // one does, without opening.
    const from = indexOfValue(props.options, props.modelValue)
    const i = typeaheadIndex(props.options, e.key, from)
    if (i >= 0) choose(i)
  }
}

function onListKey(e) {
  switch (e.key) {
    case 'ArrowDown':
      e.preventDefault()
      active.value = nextEnabledIndex(props.options, active.value, 1)
      scrollActiveIntoView()
      break
    case 'ArrowUp':
      e.preventDefault()
      active.value = nextEnabledIndex(props.options, active.value, -1)
      scrollActiveIntoView()
      break
    case 'Home':
      e.preventDefault()
      active.value = firstEnabledIndex(props.options)
      scrollActiveIntoView()
      break
    case 'End':
      e.preventDefault()
      active.value = lastEnabledIndex(props.options)
      scrollActiveIntoView()
      break
    case 'Enter':
    case ' ':
      e.preventDefault()
      choose(active.value)
      break
    case 'Escape':
      e.preventDefault()
      // Escape must not change the value, and must not reach a dialog behind
      // the menu and close that instead.
      e.stopPropagation()
      closeMenu()
      break
    case 'Tab':
      closeMenu({ refocus: false })
      break
    default:
      if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
        e.preventDefault()
        onTypeahead(e.key)
      }
  }
}

// --- dismissal --------------------------------------------------------------

function onDocPointerDown(e) {
  if (!open.value) return
  if (triggerEl.value?.contains(e.target) || listEl.value?.contains(e.target)) return
  closeMenu({ refocus: false })
}

function onReflow(e) {
  if (!open.value) return
  // The listener is capturing, so it also sees the menu's own scrolling.
  // Repositioning then would fight the user for control of the list.
  if (e?.target && listEl.value?.contains(e.target)) return
  place()
}

watch(open, isOpen => {
  const method = isOpen ? 'addEventListener' : 'removeEventListener'
  document[method]('pointerdown', onDocPointerDown, true)
  // The menu is fixed, so it does not travel with a scrolling panel: follow
  // the trigger rather than leaving the list stranded.
  window[method]('scroll', onReflow, true)
  window[method]('resize', onReflow)
})

onBeforeUnmount(() => {
  clearTimeout(queryTimer)
  document.removeEventListener('pointerdown', onDocPointerDown, true)
  window.removeEventListener('scroll', onReflow, true)
  window.removeEventListener('resize', onReflow)
})
</script>
