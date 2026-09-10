/**
 * A stable colour per host group.
 *
 * The previous version applied its colour only on `hover:`, which meant group
 * colour was invisible at rest and did no work at all — you had to point at a
 * group to learn its hue. These are rest-visible rails instead.
 *
 * Written as whole class names, not built by interpolation, because Tailwind
 * scans source text: `border-tag-${n}` would compile to nothing.
 */

/** Eight hues, each clearing 3:1 against the sidebar surface in both themes. */
export const GROUP_ACCENTS = [
  'border-tag-1',
  'border-tag-2',
  'border-tag-3',
  'border-tag-4',
  'border-tag-5',
  'border-tag-6',
  'border-tag-7',
  'border-tag-8',
]

/**
 * Which hue a group name gets, 0..7.
 *
 * Hashed rather than assigned in order so a group keeps its colour when the
 * ones above it are renamed, deleted or re-sorted — the colour is a property
 * of the name, not of the position.
 */
export function groupAccentIndex(name) {
  const s = String(name ?? '')
  let hash = 0
  for (let i = 0; i < s.length; i++) {
    hash = (hash << 5) - hash + s.charCodeAt(i)
    hash |= 0
  }
  return Math.abs(hash) % GROUP_ACCENTS.length
}

/** The border class for a group's rail. */
export function groupAccentClass(name) {
  return GROUP_ACCENTS[groupAccentIndex(name)]
}
