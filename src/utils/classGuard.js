/**
 * Rules that keep the design system from leaking.
 *
 * The colour pass replaced 1,046 hardcoded hex values with tokens; within two
 * commits, 45 raw Tailwind palette classes had appeared behind them — several
 * of them mine. Vigilance does not scale across 39 components, so the rules
 * are asserted by a test instead.
 *
 * Each phase of the UI pass widens BANNED. Pure and file-agnostic: the test
 * supplies the source text, so this needs no DOM and no build step.
 */

/** Every hue Tailwind ships, so a rule cannot miss one by omission. */
const HUES = [
  'slate', 'gray', 'zinc', 'neutral', 'stone', 'red', 'orange', 'amber',
  'yellow', 'lime', 'green', 'emerald', 'teal', 'cyan', 'sky', 'blue',
  'indigo', 'violet', 'purple', 'fuchsia', 'pink', 'rose',
]

export const BANNED = [
  {
    name: 'palette-colour',
    // Any utility naming a Tailwind hue directly rather than a role token.
    pattern: new RegExp(
      `\\b(bg|text|border|ring|divide|from|via|to|outline|shadow|accent|caret|fill|stroke|placeholder)-(${HUES.join('|')})-(50|\\d00)\\b`,
      'g',
    ),
    why: 'names a Tailwind hue instead of a role token, so it ignores the theme',
    fix: 'use a token from src/themes/tokens.css (ink-2, bad, good-solid, tag-3, …)',
  },
  {
    name: 'bright-fill-white-label',
    // `accent`/`good`/`warn`/`bad` are text tones. White on the bright accent
    // is 2.79:1 -- the *-solid variants exist precisely to carry a label.
    pattern: /\bbg-(accent|good|warn|bad|mongo)(?![\w-])(?=[^"']*\btext-(white|black)\b)/g,
    why: 'a bright tone cannot carry a white or black label (accent + white is 2.79:1)',
    fix: 'use the -solid variant for fills: bg-accent-solid, bg-good-solid, …',
  },
  {
    name: 'arbitrary-type-size',
    // A bracket size sets a size and nothing else, so it inherits preflight's
    // 1.5 line-height. That is where the host row lost ~7px per host.
    pattern: /\btext-\[\d+px\]/g,
    why: 'sets a font size with no paired line-height, so leading falls back to 1.5',
    fix: 'use a scale tier: text-2xs, text-xs, text-sm, text-base, text-lg',
  },
]

/**
 * Every banned class in one file's source.
 *
 * Returns line numbers so a failure names the place, not just the file.
 */
export function findBannedClasses(source, rules = BANNED) {
  const found = []
  const lines = String(source ?? '').split('\n')
  for (const rule of rules) {
    lines.forEach((line, i) => {
      // Fresh lastIndex per line: these are /g regexes and are reused.
      rule.pattern.lastIndex = 0
      let m
      while ((m = rule.pattern.exec(line)) !== null) {
        found.push({ rule: rule.name, match: m[0], line: i + 1, why: rule.why, fix: rule.fix })
        if (m.index === rule.pattern.lastIndex) rule.pattern.lastIndex++
      }
    })
  }
  return found
}

/** A readable failure message for a set of findings. */
export function formatFindings(file, findings) {
  return findings.map(f => `  ${file}:${f.line}  ${f.match}\n      ${f.why}\n      → ${f.fix}`).join('\n')
}
