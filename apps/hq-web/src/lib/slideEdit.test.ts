import { describe, expect, test } from 'bun:test'
import { readHqmd } from './hqmd'
import { addCard, cardSpans, contrastRatio, contrastWarning, deleteCard, duplicateCard, moveCard, parseColor, parseTransition, setCardMeta } from './slideEdit'

const DECK = '---\nsuite_kind: slides\n---\n# Deck\n\nIntro\n\n## One\n\nHello\n\n## Two\n<!-- layout: two-column -->\nWorld\n\n```\n## not a card\n```\n\n## Three\n\nBye\n'
const titles = (text: string) => readHqmd(text).slides.map((s) => s.title)

describe('cards', () => {
  test('a heading inside code is not a card', () => {
    expect(cardSpans(DECK.split('\n')).length).toBe(3)
    expect(titles(DECK)).toEqual(['One', 'Two', 'Three'])
  })
  test('add puts a new card after the nth (-1 is before the first), or into an empty deck', () => {
    expect(titles(addCard(DECK, 0, 'Extra')!)).toEqual(['One', 'Extra', 'Two', 'Three'])
    expect(titles(addCard(DECK, -1, '')!)).toEqual(['New card', 'One', 'Two', 'Three'])
    expect(titles(addCard(DECK, 2, '')!)).toEqual(['One', 'Two', 'Three', 'New card'])
    expect(titles(addCard('---\nsuite_kind: slides\n---\n# Deck\n', -1, 'First')!)).toEqual(['First'])
    expect(addCard(DECK, 9, 'x')).toBeNull()
  })
  test('copy and delete; a delete names the title it saw', () => {
    expect(titles(duplicateCard(DECK, 1)!)).toEqual(['One', 'Two', 'Two', 'Three'])
    expect(readHqmd(duplicateCard(DECK, 1)!).slides[2].layout).toBe('two-column')
    expect(titles(deleteCard(DECK, 1, 'Two')!)).toEqual(['One', 'Three'])
    expect(deleteCard(DECK, 1, 'Zwei')).toBeNull()
    expect(deleteCard(DECK, 5, 'Two')).toBeNull()
  })
  test('move reorders the cards and keeps what is above the first card', () => {
    const moved = moveCard(DECK, 2, 0)!
    expect(titles(moved)).toEqual(['Three', 'One', 'Two'])
    expect(moved).toContain('# Deck\n\nIntro\n')
    expect(readHqmd(moved).slides[2].body).toContain('World')
    expect(moveCard(DECK, 1, 1)).toBe(DECK)
    expect(moveCard(DECK, 0, 7)).toBeNull()
    // Moving there and back gives the same cards.
    expect(readHqmd(moveCard(moved, 0, 2)!).slides).toEqual(readHqmd(DECK).slides)
  })
})

describe('card comments', () => {
  test('set, replace and remove a layout', () => {
    const a = setCardMeta(DECK, 0, 'layout', 'two-column')!
    expect(readHqmd(a).slides[0].layout).toBe('two-column')
    const b = setCardMeta(a, 1, 'layout', null)!
    expect(readHqmd(b).slides[1].layout).toBeNull()
    expect(b).not.toContain('layout: two-column -->\nWorld')
  })
  test('transition, colours and a multi-line note are read back', () => {
    let t = setCardMeta(DECK, 0, 'transition', 'push slow')!
    t = setCardMeta(t, 0, 'background', '#1F3A5F')!
    t = setCardMeta(t, 0, 'color', '#FFFFFF')!
    t = setCardMeta(t, 0, 'notes', 'Say hello.\n\nThen pause.')!
    const s = readHqmd(t).slides[0]
    expect(s).toMatchObject({ title: 'One', body: 'Hello', transition: 'push slow', background: '#1F3A5F', color: '#FFFFFF', notes: 'Say hello.\n\nThen pause.' })
    // Changing one leaves the others.
    const u = readHqmd(setCardMeta(t, 0, 'transition', 'fade')!).slides[0]
    expect(u).toMatchObject({ transition: 'fade', background: '#1F3A5F', notes: 'Say hello.\n\nThen pause.' })
    expect(readHqmd(setCardMeta(t, 0, 'notes', null)!).slides[0].notes).toBeNull()
  })
  test('text that would close the comment cannot break out of it', () => {
    const s = readHqmd(setCardMeta(DECK, 0, 'notes', 'a --> <b>x</b> <!-- c')!).slides[0]
    expect(s.notes).toBe('a  <b>x</b>  c')
    expect(s.body).toBe('Hello')
  })
  test('the deck-level look is read from before the first card', () => {
    const d = readHqmd('---\nsuite_kind: slides\n---\n# D\n<!-- transition: fade -->\n<!-- background: #000000 -->\n\n## A\n\nx\n')
    expect(d.deck).toMatchObject({ transition: 'fade', background: '#000000', color: null })
  })
  test('a comment inside a code fence is code, not a setting', () => {
    const s = readHqmd('---\nsuite_kind: slides\n---\n## A\n\n```\n<!-- notes: nope -->\n```\n').slides[0]
    expect(s.notes).toBeNull()
    expect(s.body).toContain('notes: nope')
  })
})

describe('contrast rule, 3 to 1', () => {
  test('colours parse as #RGB and #RRGGBB', () => {
    expect(parseColor('#fff')).toEqual([255, 255, 255])
    expect(parseColor('#1F3A5F')).toEqual([31, 58, 95])
    expect(parseColor('red')).toBeNull()
  })
  test('black on white is 21 to 1', () => {
    expect(Math.round(contrastRatio([0, 0, 0], [255, 255, 255]))).toBe(21)
  })
  test('warns under 3 to 1, counting the theme white and black for what is unset', () => {
    expect(contrastWarning('#777777', '#808080')).toBeLessThan(3)
    expect(contrastWarning('#FFFFFF', '#1F3A5F')).toBeNull()
    // Unset text is black: black on a dark blue is hard to read.
    expect(contrastWarning(null, '#1F3A5F')).not.toBeNull()
    // Unset background is white: white text on it is unreadable.
    expect(contrastWarning('#FFFFFF', null)).not.toBeNull()
    expect(contrastWarning(null, null)).toBeNull()
    expect(contrastWarning('nonsense', '#000000')).toBeNull()
  })
  test('a transition is a known name and an optional speed', () => {
    expect(parseTransition('push slow')).toEqual({ name: 'push', speed: 'slow' })
    expect(parseTransition('fade')).toEqual({ name: 'fade', speed: null })
    expect(parseTransition('spin')).toBeNull()
    expect(parseTransition(null)).toBeNull()
  })
})
