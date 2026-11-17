import { describe, expect, test } from 'bun:test'
import { REPLY_FOLDER, makeTaskDraft, saveReplyToVault } from './saveReply'

const fakes = () => {
  const calls: unknown[][] = []
  const removed: string[] = []
  return {
    calls,
    removed,
    deps: {
      create: async (...args: unknown[]) => {
        calls.push(args)
        return { success: true, path: 'Notebooks/Inbox/Samsung battery steps.md' }
      },
      remove: async (path: string) => {
        removed.push(path)
        return { success: true, trash_path: 'x' }
      },
    } as never,
  }
}

describe('saving a reply to the Vault', () => {
  test('names the note by the reply, puts it in the Inbox folder and keys the create by the message', async () => {
    const f = fakes()
    const saved = await saveReplyToVault('## Samsung battery steps\n\nOpen Settings, then Battery.', 'm-42', f.deps)
    expect(f.calls).toEqual([['Samsung battery steps', '## Samsung battery steps\n\nOpen Settings, then Battery.', REPLY_FOLDER, 'chat-m-42']])
    expect(saved.path).toBe('Notebooks/Inbox/Samsung battery steps.md')
  })

  test('Undo puts the note in the Trash', async () => {
    const f = fakes()
    const saved = await saveReplyToVault('A short reply.', 'm-1', f.deps)
    await saved.undo()
    expect(f.removed).toEqual(['Notebooks/Inbox/Samsung battery steps.md'])
  })

  test('a reply with no words to name it by is called Chat reply', async () => {
    const f = fakes()
    await saveReplyToVault('```\ncode only\n```', 'm-2', f.deps)
    expect(f.calls[0][0]).toBe('Chat reply')
  })

  test('a failed create is the caller\'s to report', async () => {
    const deps = { create: async () => { throw new Error('offline') }, remove: async () => ({}) } as never
    await expect(saveReplyToVault('x', 'm-3', deps)).rejects.toThrow('offline')
  })
})

describe('Make a task from a reply', () => {
  test('is a request in plain words, naming the reply, for the person to send or change', () => {
    expect(makeTaskDraft('I wrote the steps down. There are four.')).toBe('Make a task from your answer: I wrote the steps down')
    expect(makeTaskDraft('')).toBe('Make a task from your answer: Chat reply')
  })
})
