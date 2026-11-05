import { expect, test } from 'bun:test'
import { emptyChatChips, type Capability, type CapabilitiesView } from './capabilitiesApi'

const cap = (id: string, over: Partial<Capability> = {}): Capability => ({
  id,
  group: 'notes',
  says: `Do ${id}`,
  example: `Try ${id}`,
  state: 'ready',
  used: false,
  ...over,
})

const view = (suggestions: Capability[], needs_model = false): CapabilitiesView => ({
  suggestions,
  capabilities: suggestions,
  needs_model,
})

test('the empty chat shows ready suggestions with an example, in the order given', () => {
  const chips = emptyChatChips(view([cap('a'), cap('b', { example: ' ' }), cap('c'), cap('d', { state: 'one_step' })]))
  expect(chips.map((c) => c.id)).toEqual(['a', 'c'])
})

test('no chips before a model key, and none while loading', () => {
  expect(emptyChatChips(view([cap('a')], true))).toEqual([])
  expect(emptyChatChips(null)).toEqual([])
  expect(emptyChatChips(undefined)).toEqual([])
})

test('at most four chips', () => {
  const many = ['a', 'b', 'c', 'd', 'e', 'f'].map((id) => cap(id))
  expect(emptyChatChips(view(many))).toHaveLength(4)
})
