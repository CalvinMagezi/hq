import { expect, test } from 'bun:test'
import { sanitizeVaultPath, useVaultNoteStore } from './vaultNoteStore'

test('sanitizeVaultPath accepts valid vault references and strips decorators', () => {
  expect(sanitizeVaultPath('Notebooks/Inbox/Plan.md')).toBe('Notebooks/Inbox/Plan.md')
  expect(sanitizeVaultPath('Notebooks/Inbox/Plan')).toBe('Notebooks/Inbox/Plan')
  expect(sanitizeVaultPath('[[My Note]]')).toBe('My Note')
  expect(sanitizeVaultPath('[[Projects/Alpha|Alpha Project]]')).toBe('Projects/Alpha')
  expect(sanitizeVaultPath('vault:Notebooks/Spec.md')).toBe('Notebooks/Spec.md')
  expect(sanitizeVaultPath('note:Notebooks/Spec.md')).toBe('Notebooks/Spec.md')
  expect(sanitizeVaultPath('/api/note?path=Notebooks%2FInbox%2FPlan.md')).toBe('Notebooks/Inbox/Plan.md')
})

test('sanitizeVaultPath rejects external URLs, protocol links, and directory traversal', () => {
  expect(sanitizeVaultPath('https://example.com')).toBeNull()
  expect(sanitizeVaultPath('http://malicious.org/note.md')).toBeNull()
  expect(sanitizeVaultPath('javascript:alert(1)')).toBeNull()
  expect(sanitizeVaultPath('//cdn.example.com/file')).toBeNull()
  expect(sanitizeVaultPath('../etc/passwd')).toBeNull()
  expect(sanitizeVaultPath('Notebooks/../../secret.txt')).toBeNull()
  expect(sanitizeVaultPath('/etc/passwd')).toBeNull()
  expect(sanitizeVaultPath('')).toBeNull()
})

test('useVaultNoteStore opens and closes drawer with valid path', () => {
  const store = useVaultNoteStore.getState()
  store.closeNote()
  expect(useVaultNoteStore.getState().open).toBe(false)
  expect(useVaultNoteStore.getState().notePath).toBeNull()

  store.openNote('[[Notebooks/Inbox/Plan.md]]')
  expect(useVaultNoteStore.getState().open).toBe(true)
  expect(useVaultNoteStore.getState().notePath).toBe('Notebooks/Inbox/Plan.md')

  store.closeNote()
  expect(useVaultNoteStore.getState().open).toBe(false)
  expect(useVaultNoteStore.getState().notePath).toBeNull()
})

test('useVaultNoteStore ignores unsafe path opens', () => {
  const store = useVaultNoteStore.getState()
  store.closeNote()

  store.openNote('https://external.com')
  expect(useVaultNoteStore.getState().open).toBe(false)
  expect(useVaultNoteStore.getState().notePath).toBeNull()

  store.openNote('../secret')
  expect(useVaultNoteStore.getState().open).toBe(false)
  expect(useVaultNoteStore.getState().notePath).toBeNull()
})
