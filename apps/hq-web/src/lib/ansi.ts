// Pure ANSI SGR parser for the Workbench terminal. Only colors and text attributes are kept;
// every other escape sequence and stray control character is dropped.

/** A palette index (0-255) or a literal [r, g, b]. */
export type AnsiColor = number | readonly [number, number, number]

export interface AnsiStyle {
  fg?: AnsiColor
  bg?: AnsiColor
  bold?: boolean
  dim?: boolean
  italic?: boolean
  underline?: boolean
  inverse?: boolean
}

export interface AnsiSpan {
  text: string
  style: AnsiStyle
}

/** Longest line we parse; the rest is dropped so a pathological line cannot stall rendering. */
export const MAX_LINE_CHARS = 4_000
/** An escape sequence longer than this without a terminator is treated as garbage. */
const MAX_SEQUENCE_CHARS = 64
const MAX_SGR_PARAMS = 32

const ESC = 0x1b
const BEL = 0x07
const TAB = 0x09
const DEL = 0x7f
const FIRST_PRINTABLE = 0x20
const CSI_FINAL_MIN = 0x40
const CSI_FINAL_MAX = 0x7e
const PALETTE_SIZE = 256
const ANSI_16 = 16
const CUBE_START = 16
const GRAY_START = 232
const CUBE_SIDE = 6
const CUBE_STEP = 40
const CUBE_BASE = 55
const GRAY_BASE = 8
const GRAY_STEP = 10
const BRIGHT_OFFSET = 8
const RGB_MAX = 255

function sgrColor(params: number[], at: number): { color?: AnsiColor; used: number } {
  const mode = params[at + 1]
  if (mode === 5) {
    const n = params[at + 2]
    return n === undefined ? { used: 2 } : { color: Math.min(Math.max(n, 0), PALETTE_SIZE - 1), used: 3 }
  }
  if (mode === 2) {
    const rgb = params.slice(at + 2, at + 5)
    if (rgb.length < 3) return { used: params.length - at }
    return { color: rgb.map((v) => Math.min(Math.max(v, 0), RGB_MAX)) as unknown as AnsiColor, used: 5 }
  }
  return { used: 1 }
}

/** Colon sub-parameters (38:2::r:g:b, 38:5:n) are folded into the semicolon form. */
function readParams(raw: string): number[] {
  const out: number[] = []
  for (const token of raw.split(';').slice(0, MAX_SGR_PARAMS)) {
    if (!token.includes(':')) {
      out.push(Number(token) || 0)
      continue
    }
    const sub = token.split(':').map((p) => Number(p) || 0)
    if ((sub[0] === 38 || sub[0] === 48) && sub[1] === 2) out.push(sub[0], 2, ...sub.slice(-3))
    else out.push(...sub)
  }
  return out
}

/** Applies one SGR parameter list to a style and returns the new style. */
export function applySgr(prev: AnsiStyle, raw: string): AnsiStyle {
  const params = raw === '' ? [0] : readParams(raw)
  const s: AnsiStyle = { ...prev }
  for (let i = 0; i < params.length; i++) {
    const p = params[i]
    if (p === 0) {
      for (const k of Object.keys(s) as (keyof AnsiStyle)[]) delete s[k]
    } else if (p === 1) s.bold = true
    else if (p === 2) s.dim = true
    else if (p === 3) s.italic = true
    else if (p === 4) s.underline = true
    else if (p === 7) s.inverse = true
    else if (p === 22) {
      delete s.bold
      delete s.dim
    } else if (p === 23) delete s.italic
    else if (p === 24) delete s.underline
    else if (p === 27) delete s.inverse
    else if (p >= 30 && p <= 37) s.fg = p - 30
    else if (p >= 90 && p <= 97) s.fg = p - 90 + BRIGHT_OFFSET
    else if (p >= 40 && p <= 47) s.bg = p - 40
    else if (p >= 100 && p <= 107) s.bg = p - 100 + BRIGHT_OFFSET
    else if (p === 39) delete s.fg
    else if (p === 49) delete s.bg
    else if (p === 38 || p === 48) {
      const { color, used } = sgrColor(params, i)
      if (color !== undefined) s[p === 38 ? 'fg' : 'bg'] = color
      i += used - 1
    }
  }
  return s
}

function sameStyle(a: AnsiStyle, b: AnsiStyle): boolean {
  const colorKey = (c?: AnsiColor) => (c === undefined ? '' : typeof c === 'number' ? String(c) : c.join(','))
  return (
    colorKey(a.fg) === colorKey(b.fg) &&
    colorKey(a.bg) === colorKey(b.bg) &&
    !!a.bold === !!b.bold &&
    !!a.dim === !!b.dim &&
    !!a.italic === !!b.italic &&
    !!a.underline === !!b.underline &&
    !!a.inverse === !!b.inverse
  )
}

/** Index just past an OSC sequence starting at `from` (after ESC ]), ended by BEL or ESC \. */
function skipOsc(line: string, from: number): number {
  const limit = Math.min(line.length, from + MAX_SEQUENCE_CHARS * 16)
  for (let i = from; i < limit; i++) {
    const c = line.charCodeAt(i)
    if (c === BEL) return i + 1
    if (c === ESC && line[i + 1] === '\\') return i + 2
  }
  return from
}

/** Parses one line (no newlines) into spans. Style starts reset on every line. */
export function parseAnsiLine(input: string): AnsiSpan[] {
  const line = input.length > MAX_LINE_CHARS ? input.slice(0, MAX_LINE_CHARS) : input
  const spans: AnsiSpan[] = []
  let style: AnsiStyle = {}
  let text = ''
  const flush = () => {
    if (!text) return
    const last = spans[spans.length - 1]
    if (last && sameStyle(last.style, style)) last.text += text
    else spans.push({ text, style })
    text = ''
  }
  let i = 0
  while (i < line.length) {
    const c = line.charCodeAt(i)
    if (c !== ESC) {
      if (c === TAB || (c >= FIRST_PRINTABLE && c !== DEL)) text += line[i]
      i++
      continue
    }
    const kind = line[i + 1]
    if (kind === '[') {
      const end = Math.min(line.length, i + 2 + MAX_SEQUENCE_CHARS)
      let j = i + 2
      while (j < end && (line.charCodeAt(j) < CSI_FINAL_MIN || line.charCodeAt(j) > CSI_FINAL_MAX)) j++
      if (j >= end) {
        i++
        continue
      }
      if (line[j] === 'm') {
        flush()
        style = applySgr(style, line.slice(i + 2, j))
      }
      i = j + 1
    } else if (kind === ']') {
      const next = skipOsc(line, i + 2)
      i = next === i + 2 ? i + 1 : next
    } else if (kind !== undefined && '()*+'.includes(kind)) i += 3
    else i += kind === undefined ? 1 : 2
  }
  flush()
  return spans
}

/** Parses whole text; one span list per line. */
export function parseAnsi(text: string): AnsiSpan[][] {
  return text.split('\n').map(parseAnsiLine)
}

/** The text with every escape sequence removed. */
export function stripAnsi(text: string): string {
  return parseAnsi(text)
    .map((spans) => spans.map((s) => s.text).join(''))
    .join('\n')
}

/** The 6x6x6 cube and grayscale ramp as rgb; null for the 16 theme-mapped colors. */
export function paletteRgb(index: number): [number, number, number] | null {
  if (index < ANSI_16 || index >= PALETTE_SIZE) return null
  if (index >= GRAY_START) {
    const v = GRAY_BASE + (index - GRAY_START) * GRAY_STEP
    return [v, v, v]
  }
  const n = index - CUBE_START
  const level = (v: number) => (v === 0 ? 0 : CUBE_BASE + v * CUBE_STEP)
  return [level(Math.floor(n / (CUBE_SIDE * CUBE_SIDE))), level(Math.floor(n / CUBE_SIDE) % CUBE_SIDE), level(n % CUBE_SIDE)]
}

const SRGB_LINEAR_CUTOFF = 0.03928
const LUM_R = 0.2126
const LUM_G = 0.7152
const LUM_B = 0.0722

function luminance([r, g, b]: readonly [number, number, number]): number {
  const lin = (v: number) => {
    const x = v / RGB_MAX
    return x <= SRGB_LINEAR_CUTOFF ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4
  }
  return LUM_R * lin(r) + LUM_G * lin(g) + LUM_B * lin(b)
}

/** WCAG contrast ratio of a color against black. */
export function contrastOnBlack(rgb: readonly [number, number, number]): number {
  return (luminance(rgb) + 0.05) / 0.05
}

/** Readable text on the dark terminal; AA for normal text. */
export const MIN_CONTRAST = 4.5
const LIGHTEN_STEPS = 20

/** Lightens a color toward white until it reaches the minimum contrast against black. */
export function ensureContrast(rgb: readonly [number, number, number], min = MIN_CONTRAST): [number, number, number] {
  const base: [number, number, number] = [rgb[0], rgb[1], rgb[2]]
  for (let step = 0; step <= LIGHTEN_STEPS; step++) {
    const t = step / LIGHTEN_STEPS
    const mixed = base.map((v) => Math.round(v + (RGB_MAX - v) * t)) as [number, number, number]
    if (contrastOnBlack(mixed) >= min) return mixed
  }
  return [RGB_MAX, RGB_MAX, RGB_MAX]
}

const DEFAULT_FG = 'var(--ansi-7)'
const DEFAULT_BG_FOR_INVERSE = 'var(--bg-base)'
const DIM_OPACITY = 0.6

function cssColor(color: AnsiColor, guard: boolean): string {
  const rgb = typeof color === 'number' ? paletteRgb(color) : color
  if (rgb === null) return `var(--ansi-${color})`
  const [r, g, b] = guard ? ensureContrast(rgb) : rgb
  return `rgb(${r}, ${g}, ${b})`
}

export interface AnsiCss {
  color?: string
  backgroundColor?: string
  fontWeight?: number
  fontStyle?: 'italic'
  textDecoration?: 'underline'
  opacity?: number
}

/** Inline style for a span. Foreground colors are contrast-guarded; backgrounds are as given. */
export function spanCss(style: AnsiStyle): AnsiCss {
  const css: AnsiCss = {}
  if (style.inverse) {
    css.color = style.bg === undefined ? DEFAULT_BG_FOR_INVERSE : cssColor(style.bg, false)
    css.backgroundColor = style.fg === undefined ? DEFAULT_FG : cssColor(style.fg, false)
  } else {
    if (style.fg !== undefined) css.color = cssColor(style.fg, true)
    if (style.bg !== undefined) css.backgroundColor = cssColor(style.bg, false)
  }
  if (style.bold) css.fontWeight = 700
  if (style.italic) css.fontStyle = 'italic'
  if (style.underline) css.textDecoration = 'underline'
  if (style.dim) css.opacity = DIM_OPACITY
  return css
}
