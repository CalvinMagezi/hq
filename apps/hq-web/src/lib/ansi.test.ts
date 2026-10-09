import { expect, test } from 'bun:test'
import { MAX_LINE_CHARS, MIN_CONTRAST, MIN_PAIR_CONTRAST, contrastRatio, plainText, contrastOnBlack, ensureContrast, paletteRgb, parseAnsi, parseAnsiLine, spanCss, stripAnsi } from './ansi'

const E = '\x1b['

test('plain text is one unstyled span', () => {
  expect(parseAnsiLine('hello')).toEqual([{ text: 'hello', style: {} }])
})

test('basic and bright colors map to palette indexes', () => {
  const [a, b, c, d] = parseAnsiLine(`${E}31mr${E}92mg${E}44mb${E}0mn`)
  expect(a.style.fg).toBe(1)
  expect(b.style.fg).toBe(10)
  expect(c.style).toEqual({ fg: 10, bg: 4 })
  expect(d.style).toEqual({})
})

test('bold, dim, italic, underline, inverse set and clear individually', () => {
  const spans = parseAnsiLine(`${E}1;3;4;7mx${E}22my${E}23;24;27mz`)
  expect(spans[0].style).toEqual({ bold: true, italic: true, underline: true, inverse: true })
  expect(spans[1].style).toEqual({ italic: true, underline: true, inverse: true })
  expect(spans[2].style).toEqual({})
  expect(parseAnsiLine(`${E}2mx`)[0].style.dim).toBe(true)
})

test('39 and 49 reset only the color', () => {
  const spans = parseAnsiLine(`${E}1;31;42ma${E}39mb${E}49mc`)
  expect(spans[1].style).toEqual({ bold: true, bg: 2 })
  expect(spans[2].style).toEqual({ bold: true })
})

test('256-color and truecolor, semicolon and colon forms', () => {
  expect(parseAnsiLine(`${E}38;5;196mx`)[0].style.fg).toBe(196)
  expect(parseAnsiLine(`${E}48;5;21mx`)[0].style.bg).toBe(21)
  expect(parseAnsiLine(`${E}38;2;10;20;30mx`)[0].style.fg).toEqual([10, 20, 30])
  expect(parseAnsiLine(`${E}48;2;1;2;3mx`)[0].style.bg).toEqual([1, 2, 3])
  expect(parseAnsiLine(`${E}38:2::9:8:7mx`)[0].style.fg).toEqual([9, 8, 7])
  expect(parseAnsiLine(`${E}38:5:44mx`)[0].style.fg).toBe(44)
  expect(parseAnsiLine(`${E}38;2;999;0;0mx`)[0].style.fg).toEqual([255, 0, 0])
})

test('a truncated extended color does not eat the following codes', () => {
  expect(parseAnsiLine(`${E}38;5mx`)[0].style).toEqual({})
  expect(parseAnsiLine(`${E}38;2;1;2mx`)[0].style).toEqual({})
})

test('same-style neighbours merge', () => {
  expect(parseAnsiLine(`${E}31ma${E}31mb`)).toEqual([{ text: 'ab', style: { fg: 1 } }])
})

test('cursor, erase, OSC and other escapes are stripped', () => {
  expect(stripAnsi(`${E}2K${E}1;1Hhi${E}?25l \x1b]0;title\x07there\x1b]8;;http://x\x1b\\!\x1b(B`)).toBe('hi there!')
})

test('control characters are dropped but tabs stay', () => {
  expect(stripAnsi('a\x00b\x07c\r\td\x7f')).toBe('abc\td')
})

test('lines split on newline', () => {
  expect(parseAnsi(`${E}31ma\nb`).map((l) => l.map((s) => s.text))).toEqual([['a'], ['b']])
})

test('pathological input is bounded and does not throw', () => {
  const long = parseAnsiLine('x'.repeat(MAX_LINE_CHARS * 10))
  expect(long[0].text.length).toBe(MAX_LINE_CHARS)
  expect(() => parseAnsiLine(`${E}${'1;'.repeat(50_000)}`)).not.toThrow()
  expect(parseAnsiLine('\x1b'.repeat(5000)).length).toBe(0)
  expect(stripAnsi(`${E}${'9'.repeat(1000)}text`)).toContain('text')
  const many = parseAnsiLine(`${E}31ma${E}32mb`.repeat(MAX_LINE_CHARS))
  expect(many.length).toBeGreaterThan(0)
})

test('palette: cube and grayscale values, theme colors are not computed', () => {
  expect(paletteRgb(15)).toBeNull()
  expect(paletteRgb(16)).toEqual([0, 0, 0])
  expect(paletteRgb(196)).toEqual([255, 0, 0])
  expect(paletteRgb(231)).toEqual([255, 255, 255])
  expect(paletteRgb(232)).toEqual([8, 8, 8])
  expect(paletteRgb(255)).toEqual([238, 238, 238])
})

test('contrast guard lightens dark colors and leaves readable ones alone', () => {
  expect(ensureContrast([0, 0, 0])).not.toEqual([0, 0, 0])
  expect(contrastOnBlack(ensureContrast([0, 0, 0]))).toBeGreaterThanOrEqual(MIN_CONTRAST)
  expect(contrastOnBlack(ensureContrast([0, 0, 128]))).toBeGreaterThanOrEqual(MIN_CONTRAST)
  expect(ensureContrast([255, 200, 0])).toEqual([255, 200, 0])
})

test('spanCss maps theme colors to variables and guards program colors', () => {
  expect(spanCss({ fg: 1 }).color).toBe('var(--ansi-1)')
  expect(spanCss({ fg: 16 }).color).not.toBe('rgb(0, 0, 0)')
  expect(spanCss({ bg: 16 }).backgroundColor).toBe('rgb(0, 0, 0)')
  expect(spanCss({ bold: true, italic: true, underline: true, dim: true })).toMatchObject({ fontWeight: 700, fontStyle: 'italic', textDecoration: 'underline', opacity: 0.6 })
})

test('inverse swaps foreground and background', () => {
  const css = spanCss({ inverse: true, fg: 7, bg: 0 })
  expect(css.color).toBe('var(--ansi-0)')
  expect(css.backgroundColor).toBe('var(--ansi-7)')
})

const rgbOfCss = (c: string) => (c.match(/\d+/g) ?? []).map(Number) as [number, number, number]

test('text equal to its background is made readable', () => {
  const css = spanCss({ fg: [10, 10, 10], bg: [10, 10, 10] })
  expect(contrastRatio(rgbOfCss(css.color!), [10, 10, 10])).toBeGreaterThanOrEqual(MIN_PAIR_CONTRAST)
  expect(spanCss({ fg: 4, bg: 4 }).color).not.toBe('var(--ansi-4)')
})

test('inverse with equal colors is also made readable', () => {
  const css = spanCss({ inverse: true, fg: [200, 200, 200], bg: [205, 205, 205] })
  expect(contrastRatio(rgbOfCss(css.color!), [200, 200, 200])).toBeGreaterThanOrEqual(MIN_PAIR_CONTRAST)
})

test('a light background gets dark text', () => {
  const css = spanCss({ bg: [250, 250, 250] })
  expect(contrastRatio(rgbOfCss(css.color!), [250, 250, 250])).toBeGreaterThanOrEqual(MIN_PAIR_CONTRAST)
})

test('readable pairs keep their theme variables', () => {
  expect(spanCss({ fg: 0, bg: 7 }).color).toBe('var(--ansi-0)')
  expect(spanCss({ fg: 2 }).color).toBe('var(--ansi-2)')
})

test('plainText of parsed rows matches stripAnsi', () => {
  const t = `${E}31ma${E}0m\nb`
  expect(plainText(parseAnsi(t))).toBe(stripAnsi(t))
})

test('many unterminated OSC introducers parse in linear time', () => {
  const text = Array.from({ length: 200 }, () => '\x1b]'.repeat(2000)).join('\n')
  const t0 = performance.now()
  parseAnsi(text)
  stripAnsi(text)
  expect(performance.now() - t0).toBeLessThan(50)
})

test('100k SGR codes in one line stay bounded', () => {
  const t0 = performance.now()
  const spans = parseAnsiLine(`${E}31ma`.repeat(100_000))
  expect(performance.now() - t0).toBeLessThan(50)
  expect(spans.length).toBeGreaterThan(0)
})

test('OSC terminated by BEL or ST is dropped, an unterminated one only loses its introducer', () => {
  expect(stripAnsi('a\x1b]0;t\x07b\x1b]8;;u\x1b\\c')).toBe('abc')
  expect(stripAnsi('a\x1b]no end')).toBe('ano end')
})
