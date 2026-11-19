import { describe, expect, test } from 'bun:test'
import { countRunningHelpers, extractHelperRuns, helperStatusLabel, resolveChatHelpers } from './subagentHelpers'
import type { HelperRun } from './helperRunsApi'
import { searchHelperRuns } from './helperRunsApi'
import { useHelperRunsStore } from '~/store/helperRunsStore'
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

  test('resolveChatHelpers uses real helper runs when present and falls back to tool steps when unavailable', async () => {
    const real: HelperRun[] = [
      {
        id: 'r1',
        thread_id: 'th-1',
        task: 't1',
        role: 'analyst',
        goal: 'Compare Q3 revenue',
        state: 'stalled',
        detached: true,
        started_at: 100,
        settled_at: null,
        result_link: null,
        result_preview: null,
        error: null,
        blocker: null,
      },
      {
        id: 'r2',
        thread_id: 'th-1',
        task: 't2',
        role: 'writer',
        goal: 'Write brief',
        state: 'done',
        detached: true,
        started_at: 110,
        settled_at: 130,
        result_link: 'Notebooks/Brief.md',
        result_preview: 'Wrote Notebooks/Brief.md',
        error: null,
        blocker: null,
      },
      {
        id: 'r3',
        thread_id: 'th-1',
        task: 't3',
        role: 'checker',
        goal: 'Verify numbers',
        state: 'timed_out',
        detached: false,
        started_at: 120,
        settled_at: 150,
        result_link: null,
        result_preview: null,
        error: 'Timed out',
        blocker: null,
      },
    ]
    const steps = [
      { toolCallId: 'c1', toolName: 'spawn_subagents', status: 'done' as const, inputArgs: { blocking: false, task: { role: 'planner', prompt: 'Fallback' } } },
    ]
    const resolved = resolveChatHelpers(real, steps)
    expect(resolved.map((h) => [h.name, helperStatusLabel(h.status), h.resultLink])).toEqual([
      ['analyst', 'Needs a look', null],
      ['writer', 'Done', 'Notebooks/Brief.md'],
      ['checker', 'Failed', null],
    ])
    expect(countRunningHelpers(resolved)).toBe(0)
    expect(countRunningHelpers(resolveChatHelpers([{ ...real[0], state: 'running' }], steps))).toBe(1)

    // Fallback when endpoint is unavailable
    useHelperRunsStore.setState({ runsByThread: {}, loadedByThread: {}, unavailableByThread: {} })
    const origFetch = globalThis.fetch
    globalThis.fetch = (() => Promise.resolve(new Response('Not found', { status: 404 }))) as unknown as typeof fetch
    try {
      await useHelperRunsStore.getState().loadThreadRuns('th-old')
      expect(useHelperRunsStore.getState().unavailableByThread['th-old']).toBe(true)
      const fallback = resolveChatHelpers(useHelperRunsStore.getState().runsByThread['th-old'] ?? [], steps)
      expect(fallback).toHaveLength(1)
      expect(fallback[0].name).toBe('planner')
    } finally {
      globalThis.fetch = origFetch
    }
  })

  test('searchHelperRuns queries /api/helper-runs and returns runs and count', async () => {
    const origFetch = globalThis.fetch
    let seenUrl = ''
    globalThis.fetch = ((input: string | URL) => {
      seenUrl = String(input)
      return Promise.resolve(
        new Response(
          JSON.stringify({
            runs: [{ id: 'r1', thread_id: 'th-1', task: 't1', role: 'researcher', goal: 'Compare prices', state: 'done', detached: true, started_at: 1, settled_at: 2, result_link: null, result_preview: null, error: null, blocker: null }],
            count: 1,
          }),
          { status: 200, headers: { 'Content-Type': 'application/json' } },
        ),
      )
    }) as unknown as typeof fetch
    try {
      const res = await searchHelperRuns('prices')
      expect(seenUrl).toContain('/api/helper-runs?q=prices')
      expect(res.count).toBe(1)
      expect(res.runs[0].role).toBe('researcher')
    } finally {
      globalThis.fetch = origFetch
    }
  })
})
