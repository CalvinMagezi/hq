import { expect, test } from 'bun:test'
import { layoutBlocks, moveBlock, setSize, sizeOf } from './dashLayout'

const NOTE = [
  '---',
  'suite_kind: dashboard',
  '---',
  '# Sales',
  '',
  '```sheet name=Sales types="text,int"',
  '| month | amount |',
  '|---|---|',
  '| Jan | 10 |',
  '```',
  '',
  '```tile from=B2:B3 fn=sum label="Total size=big"',
  '```',
  '',
  'Between the blocks.',
  '',
  '```plot type=bar from=A1:B3 x=month y=amount title="By month" size=full',
  '```',
  '',
  '```tile from=B2:B3 fn=max label="Peak"',
  '```',
  '',
].join('\n')

const heads = (text: string) => layoutBlocks(text).map((b) => b.head)

test('the layout is the tiles and plots in the order they are written, with the size each asks for', () => {
  const blocks = layoutBlocks(NOTE)
  expect(blocks.map((b) => b.type)).toEqual(['tile', 'plot', 'tile'])
  expect(blocks.map(sizeOf)).toEqual(['small', 'full', 'small'])
})

test('a plot with no size is wide, and a size the viewer does not know counts as none', () => {
  const [plot] = layoutBlocks('```plot type=bar from=A1:B3 x=m y=v\n```\n```plot type=bar from=A1:B3 x=m y=v size=huge\n```\n')
  expect(sizeOf(plot)).toBe('wide')
  expect(sizeOf(layoutBlocks('```plot type=bar from=A1:B3 x=m y=v size=huge\n```\n')[0])).toBe('wide')
})

test('moving a block later swaps places and leaves every other line as it was', () => {
  const first = heads(NOTE)[0]
  const moved = moveBlock(NOTE, 0, 1, first)
  expect(moved).not.toBeNull()
  expect(heads(moved as string).map((h) => h?.split(' ')[0])).toEqual(['```plot', '```tile', '```tile'])
  expect(moved).toContain('Between the blocks.')
  expect((moved as string).split('\n').length).toBe(NOTE.split('\n').length)
  expect((moved as string).indexOf('```sheet')).toBe(NOTE.indexOf('```sheet'))
  // Putting it back gives the note byte for byte.
  const back = moveBlock(moved as string, 1, 0, first)
  expect(back).toBe(NOTE)
})

test('moving to the end and to the start rotates the others', () => {
  const [a, , c] = heads(NOTE)
  const toEnd = moveBlock(NOTE, 0, 2, a) as string
  expect(heads(toEnd).map((h) => h?.split(' ')[0])).toEqual(['```plot', '```tile', '```tile'])
  expect(heads(toEnd)[2]).toBe(a)
  const toStart = moveBlock(NOTE, 2, 0, c) as string
  expect(heads(toStart)[0]).toBe(c)
})

test('a block that is not the one the caller saw, or a place that is not there, is refused', () => {
  expect(moveBlock(NOTE, 0, 1, '```tile from=Z1:Z2 fn=sum')).toBeNull()
  expect(moveBlock(NOTE, 0, 5, heads(NOTE)[0])).toBeNull()
  expect(moveBlock(NOTE, -1, 0, heads(NOTE)[0])).toBeNull()
  expect(moveBlock(NOTE, 1, 1, heads(NOTE)[1])).toBe(NOTE)
})

test('line endings are kept', () => {
  const crlf = NOTE.replace(/\n/g, '\r\n')
  const moved = moveBlock(crlf, 0, 1, heads(crlf)[0]) as string
  expect(moved.includes('\r\n')).toBe(true)
  expect(moved.replace(/\r\n/g, '')).not.toContain('\n')
})

test('a block with no closing fence is never moved', () => {
  const open = '```tile from=A1:A2 fn=sum\n```\n```tile from=A1:A2 fn=max\n'
  expect(moveBlock(open, 0, 1, heads(open)[0])).toBeNull()
})

test('a size is added to the fence, replaced, and removed again when it is the default', () => {
  const peak = heads(NOTE)[2]
  const wide = setSize(NOTE, 2, peak, 'wide') as string
  expect(wide).toContain('```tile from=B2:B3 fn=max label="Peak" size=wide')
  const plotHead = heads(NOTE)[1]
  const small = setSize(NOTE, 1, plotHead, 'small') as string
  expect(small).toContain('title="By month" size=small')
  expect(small).not.toContain('size=full')
  // wide is a plot's default, so it is written as no attribute.
  const plain = setSize(NOTE, 1, plotHead, 'wide') as string
  expect(plain).toContain('```plot type=bar from=A1:B3 x=month y=amount title="By month"\n')
  expect(plain).not.toContain(' size=wide')
  expect(plain).not.toContain(' size=full')
  const reset = setSize(wide, 2, heads(wide)[2], 'small') as string
  expect(reset).toBe(NOTE)
})

test('a size= inside a quoted label is not the size', () => {
  const first = heads(NOTE)[0]
  const full = setSize(NOTE, 0, first, 'full') as string
  expect(full).toContain('label="Total size=big" size=full')
  expect(setSize(NOTE, 0, first, 'small')).toBe(NOTE)
})

test('a stale head, or a block that is not there, changes nothing', () => {
  expect(setSize(NOTE, 0, '```tile from=A1:A2 fn=sum', 'wide')).toBeNull()
  expect(setSize(NOTE, 9, '```tile', 'wide')).toBeNull()
})
