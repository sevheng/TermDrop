/**
 * Terminal palettes.
 *
 * These are content colours — the ANSI set programs actually paint with — so
 * they are kept apart from the UI tokens and are not tokenised. The background
 * and foreground do track the app theme, so a light UI does not frame a black
 * terminal.
 */

const DARK = {
  background: '#0F1116',
  foreground: '#D4D8E0',
  cursor: '#D4D8E0',
  selectionBackground: '#1B3A5C',
  black: '#0F1116',
  red: '#F47067',
  green: '#8DDB8C',
  yellow: '#E3C08B',
  blue: '#7FB6EC',
  magenta: '#D3A2D8',
  cyan: '#6FD3BE',
  white: '#D4D8E0',
  brightBlack: '#8791A0',
  brightRed: '#FF8A8A',
  brightGreen: '#AFE1AE',
  brightYellow: '#F0D9AE',
  brightBlue: '#A5CDF5',
  brightMagenta: '#E3BCE6',
  brightCyan: '#96E2D2',
  brightWhite: '#EDEFF3',
}

const LIGHT = {
  background: '#FFFFFF',
  foreground: '#1B222C',
  cursor: '#1B222C',
  selectionBackground: '#CFE0F5',
  black: '#1B222C',
  red: '#B32017',
  green: '#0F5F33',
  yellow: '#7A4F00',
  blue: '#1461C4',
  magenta: '#7A3E82',
  cyan: '#136B5C',
  white: '#556070',
  brightBlack: '#5C6675',
  brightRed: '#8F1912',
  brightGreen: '#0B4D29',
  brightYellow: '#5E3D00',
  brightBlue: '#0F4E9E',
  brightMagenta: '#5F3066',
  brightCyan: '#0E5347',
  brightWhite: '#141A22',
}

export const TERMINAL_THEMES = { dark: DARK, light: LIGHT }

/** The palette for a theme name, defaulting to dark. */
export function terminalTheme(theme) {
  return TERMINAL_THEMES[theme] || DARK
}

/** Back-compat for call sites that have not been given a theme yet. */
export const TERMINAL_THEME = DARK
