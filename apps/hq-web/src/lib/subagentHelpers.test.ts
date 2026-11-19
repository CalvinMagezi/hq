import { describe, expect, test } from 'bun:test'
import { extractHelperRuns } from './subagentHelpers'
import { liveSentence, liveText } from './liveLine'
import { stepLine, stepText } from './stepWords'

describe('subagent helpers', () => {
  test('extracts multiple helpers from spawn_subagents with roles, plain goals and status', () => {
    const helpers = extractHelperRuns([
      {
        toolCallId: 'c1',
        toolName: 'spawn_subagents',
        status: 'running',
        inputArgs: {
          tasks: [
            { id: 't1', role: 'explorer', prompt: 'Check Q3 revenue numbers\nInclude details' },
            { id: 't2', role: 'coder', prompt: 'Build the HTML summary page' },
          ],
        },
      },
    ])
    expect(helpers).toEqual([
      { id: 'c1:t1', name: 'explorer', goal: 'Check Q3 revenue numbers', status: 'working', background: false },
      { id: 'c1:t2', name: 'coder', goal: 'Build the HTML summary page', status: 'working', background: false },
    ])
  })

  test('marks background-dispatched helpers so UI does not claim they finished', () => {
    const helpers = extractHelperRuns([
      {
        toolCallId: 'c2',
        toolName: 'spawn_subagents',
        status: 'done',
        inputArgs: {
          blocking: false,
          task: { role: 'planner', prompt: 'Outline next week' },
        },
      },
    ])
    expect(helpers).toEqual([
      { id: 'c2:0', name: 'planner', goal: 'Outline next week', status: 'done', background: true },
    ])
  })

  test('live working indicator names how many helpers are running and what each does', () => {
    const one = liveText(
      liveSentence([
        {
          toolName: 'spawn_subagents',
          status: 'running',
          inputArgs: { task: { role: 'explorer', prompt: 'Find the invoice' } },
        },
      ]),
    )
    expect(one).toBe('Asking explorer: Find the invoice…')

    const two = liveText(
      liveSentence([
        {
          toolName: 'spawn_subagents',
          status: 'running',
          inputArgs: {
            tasks: [
              { role: 'explorer', prompt: 'Find the invoice' },
              { role: 'coder', prompt: 'Draft the spreadsheet' },
            ],
          },
        },
      ]),
    )
    expect(two).toBe('Asking 2 helpers: explorer (Find the invoice), coder (Draft the spreadsheet)…')

    const fallback = liveText(liveSentence([{ toolName: 'spawn_subagents', status: 'running' }]))
    expect(fallback).toBe('Asking a helper to work on it…')

    expect(
      stepText(
        stepLine('spawn_subagents', {
          tasks: [
            { role: 'explorer', prompt: 'Find the invoice' },
            { role: 'coder', prompt: 'Draft the spreadsheet' },
          ],
        }),
      ),
    ).toBe('Asked 2 helpers: explorer (Find the invoice), coder (Draft the spreadsheet)')
  })
})
