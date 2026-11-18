import { describe, expect, test } from 'bun:test'
import { folderReference, helperReference, insertReference, noteReference, referenceBefore, taskReference, topLevelFolders, triggerAt } from './triggers'

describe('the @ and # triggers', () => {
  test('a word that begins with one opens a list, with what was typed after it', () => {
    expect(triggerAt('Compare @sto', 12)).toEqual({ char: '@', query: 'sto', start: 8 })
    expect(triggerAt('@', 1)).toEqual({ char: '@', query: '', start: 0 })
    expect(triggerAt('See #shop', 9)).toEqual({ char: '#', query: 'shop', start: 4 })
  })
  test('an address, a finished word or a closed reference opens nothing', () => {
    expect(triggerAt('mail me at a@b.com', 18)).toBeNull()
    expect(triggerAt('@sto and more', 13)).toBeNull()
    expect(triggerAt('plain text', 10)).toBeNull()
    expect(triggerAt('`Notebooks/A.md` ', 17)).toBeNull()
  })
  test('looks only at the text before the caret', () => {
    expect(triggerAt('Compare @sto rest', 12)?.query).toBe('sto')
  })
  test('picking replaces the word with the reference and a space, caret after', () => {
    const t = triggerAt('Compare @sto today', 12)!
    const out = insertReference('Compare @sto today', t, 12, noteReference('Notebooks/Shop/Stock rules.md'))
    expect(out.text).toBe('Compare `Notebooks/Shop/Stock rules.md` today')
    expect(out.text.slice(0, out.caret)).toBe('Compare `Notebooks/Shop/Stock rules.md` ')
    expect(insertReference('@', triggerAt('@', 1)!, 1, '`task A-1`').text).toBe('`task A-1` ')
  })
  test('says what each kind of reference looks like', () => {
    expect(folderReference('Notebooks/Shop')).toBe('`Notebooks/Shop/`')
    expect(taskReference('HQ-12')).toBe('`task HQ-12`')
    expect(helperReference('abc')).toBe('`helper abc`')
  })
})

describe('one Backspace removes a whole reference', () => {
  test('when the caret is right after one this box wrote', () => {
    const text = 'Compare `Notebooks/Shop/Stock rules.md`'
    expect(referenceBefore(text, text.length)).toEqual([8, text.length])
    expect(referenceBefore('Do `task HQ-12`', 15)).toEqual([3, 15])
  })
  test('never for code the person wrote, or across lines', () => {
    expect(referenceBefore('use `npm run`', 13)).toBeNull()
    expect(referenceBefore('`Notebooks/A\nB.md`', 17)).toBeNull()
    expect(referenceBefore('no backtick', 11)).toBeNull()
  })
})

describe('"#" with nothing typed', () => {
  test('lists the top folders and leaves system folders out', () => {
    const root = { name: 'Notebooks', path: 'Notebooks', type: 'dir' as const, children: [
      { name: '_system', path: 'Notebooks/_system', type: 'dir' as const },
      { name: 'Projects', path: 'Notebooks/Projects', type: 'dir' as const },
      { name: 'Plan.md', path: 'Notebooks/Plan.md', type: 'file' as const },
    ] }
    expect(topLevelFolders(root)).toEqual([{ path: 'Notebooks/Projects', trail: 'Projects' }])
  })
})
