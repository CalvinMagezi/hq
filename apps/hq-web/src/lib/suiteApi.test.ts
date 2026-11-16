import { expect, test } from 'bun:test'
import { suiteSourceOf } from './suiteApi'

const note = (front: string) => `---\n${front}\n---\n\nBody`

test('a note made from an office file names its source', () => {
  expect(suiteSourceOf(note('title: A\nsource: Notebooks/Inbox/Imports/A.docx\nsource_sha256: abc'))).toBe(
    'Notebooks/Inbox/Imports/A.docx',
  )
  expect(suiteSourceOf(note('source: "Notebooks/a b.docx"\nsource_sha256: abc'))).toBe('Notebooks/a b.docx')
})

test('any other note, and a note whose original is gone, has no source', () => {
  expect(suiteSourceOf('Just text')).toBeNull()
  expect(suiteSourceOf(note('title: A'))).toBeNull()
  expect(suiteSourceOf(note('source: a.docx'))).toBeNull()
  expect(suiteSourceOf(note('source: a.docx\nsource_sha256: abc\nsuite_original_state: gone'))).toBeNull()
  expect(suiteSourceOf(note('source: a.docx\nsource_sha256: abc\nsuite_original_state: trash'))).toBe('a.docx')
  expect(suiteSourceOf('---\nsource: a.docx\nsource_sha256: abc\nno end')).toBeNull()
})
