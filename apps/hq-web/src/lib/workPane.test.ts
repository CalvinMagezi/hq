import { afterEach, describe, expect, test } from 'bun:test'
import { folderLabel, readAutoOpened, shouldAutoOpen, writeAutoOpened } from './workPane'
import { useWorkPane } from '~/store/workPaneStore'

afterEach(() => useWorkPane.setState({ byThread: {} }))

describe('when the work pane opens by itself', () => {
  test('once per chat, with something made and room for it', () => {
    expect(shouldAutoOpen({ made: 'Notebooks/A.md', docked: true, already: false })).toBe(true)
    expect(shouldAutoOpen({ made: 'Notebooks/A.md', docked: true, already: true })).toBe(false)
    expect(shouldAutoOpen({ made: 'Notebooks/A.md', docked: false, already: false })).toBe(false)
    expect(shouldAutoOpen({ made: null, docked: true, already: false })).toBe(false)
  })
  test('storage that cannot be read means a chat that has not opened yet', () => {
    expect(readAutoOpened()).toEqual([])
    expect(() => writeAutoOpened(['a'])).not.toThrow()
  })
  test('says where a note was saved', () => {
    expect(folderLabel('Notebooks/Daily/Plan.md')).toBe('Vault › Daily')
    expect(folderLabel('Notebooks/Plan.md')).toBe('Vault')
  })
})

describe('the pane of each chat', () => {
  test('a chat nobody opened is closed, and closing one leaves the others alone', () => {
    const pane = useWorkPane.getState()
    expect(pane.get('a').open).toBe(false)
    pane.showNote('a', 'Notebooks/A.md')
    pane.showSource('b', 'https://example.com')
    expect(useWorkPane.getState().get('a')).toMatchObject({ open: true, tab: 'result', note: 'Notebooks/A.md' })
    expect(useWorkPane.getState().get('b')).toMatchObject({ open: true, tab: 'context', source: 'https://example.com' })
    useWorkPane.getState().close('a')
    expect(useWorkPane.getState().get('a').open).toBe(false)
    expect(useWorkPane.getState().get('b').open).toBe(true)
  })
  test('toggle flips it, and the note stays for next time', () => {
    const pane = useWorkPane.getState()
    pane.showNote('a', 'Notebooks/A.md')
    pane.toggle('a')
    expect(useWorkPane.getState().get('a').open).toBe(false)
    pane.toggle('a')
    expect(useWorkPane.getState().get('a')).toMatchObject({ open: true, note: 'Notebooks/A.md' })
  })
})
