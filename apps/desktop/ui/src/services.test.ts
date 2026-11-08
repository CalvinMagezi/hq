import { expect, test } from 'bun:test'
import { forwardNote, forwardTitle, offeredWords } from './services'

test('the services HQ offers read in plain words, never their internal names', () => {
  expect(offeredWords('gate').title).toBe('Helpers on this computer')
  expect(offeredWords('llm').title).toBe("This computer's model")
  expect(offeredWords('web').title).toBe("This computer's HQ")
  for (const name of ['gate', 'llm', 'web']) expect(offeredWords(name).title).not.toBe(name)
})

test('a service named in config.yaml keeps its name and says where it came from', () => {
  expect(offeredWords('printer')).toEqual({ title: 'printer', note: 'Set in config.yaml' })
})

test("a forward to the phone's web app is named for what it is", () => {
  expect(forwardTitle('web')).toBe("Your phone's web pages")
  expect(forwardTitle('notes-sync')).toBe('notes-sync')
  expect(forwardNote('notes-sync')).toBe('Set in config.yaml')
})
