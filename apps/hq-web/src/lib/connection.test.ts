import { describe, expect, test } from 'bun:test'
import {
  connectionState,
  retryDelaySec,
  type ConnectionInputs,
  type ConnectionState,
} from './connection'
import type { Hello } from './hello'
import { UNREACHABLE_GRACE_MS } from './reachability'

const NOW = 1_700_000_000_000

const sampleHello: Hello = {
  hq_id: 'hq-test-1',
  edition: 'full',
  version: '0.1.0',
  min_client: 1,
  starting: null,
}

const baseInputs: ConnectionInputs = {
  status401: false,
  online: true,
  helloAnsweredAt: NOW,
  lastHelloMs: NOW,
  hello: sampleHello,
  socketOpen: true,
  aiDown: false,
  proxyUpstreamGone: false,
  lite: false,
  inHub: false,
}

describe('connectionState', () => {
  test('a table of input combinations gives the seven states in order', () => {
    const cases: Array<{
      inputs: ConnectionInputs
      expectedState: ConnectionState
      expectedWords: string
      expectedDot: 'warn' | 'work' | 'off'
    }> = [
      {
        inputs: { ...baseInputs, status401: true },
        expectedState: 'signed_out',
        expectedWords: 'Signed out of this HQ',
        expectedDot: 'warn',
      },
      {
        inputs: { ...baseInputs, online: false },
        expectedState: 'offline',
        expectedWords: "You're offline",
        expectedDot: 'warn',
      },
      {
        inputs: {
          ...baseInputs,
          hello: null,
          helloAnsweredAt: NOW - UNREACHABLE_GRACE_MS,
        },
        expectedState: 'unreachable',
        expectedWords: "Your HQ can't be reached",
        expectedDot: 'warn',
      },
      {
        inputs: {
          ...baseInputs,
          hello: {
            ...sampleHello,
            starting: {
              step: 1,
              steps: 4,
              step_name: 'boot',
              started_at: NOW / 1000 - 12,
            },
          },
        },
        expectedState: 'starting',
        expectedWords: 'Starting HQ · 12 s',
        expectedDot: 'work',
      },
      {
        inputs: { ...baseInputs, socketOpen: false },
        expectedState: 'reconnecting',
        expectedWords: 'Reconnecting',
        expectedDot: 'work',
      },
      {
        inputs: { ...baseInputs, aiDown: true },
        expectedState: 'ai_down',
        expectedWords: "The AI service isn't answering",
        expectedDot: 'warn',
      },
      {
        inputs: baseInputs,
        expectedState: 'connected',
        expectedWords: 'Connected',
        expectedDot: 'off',
      },
    ]

    for (const c of cases) {
      const view = connectionState(c.inputs, NOW)
      expect(view.state).toBe(c.expectedState)
      expect(view.words).toBe(c.expectedWords)
      expect(view.dot).toBe(c.expectedDot)
    }
  })

  test('{status401: true, online: false} is signed out', () => {
    const view = connectionState({ ...baseInputs, status401: true, online: false }, NOW)
    expect(view.state).toBe('signed_out')
    expect(view.words).toBe('Signed out of this HQ')
    expect(view.dot).toBe('warn')
  })

  test('a closed socket with hello starting (started_at 12 s before now) gives Starting HQ · 12 s', () => {
    const startingHello: Hello = {
      ...sampleHello,
      starting: {
        step: 2,
        steps: 5,
        step_name: 'loading_vault',
        started_at: NOW / 1000 - 12,
      },
    }
    const view = connectionState(
      {
        ...baseInputs,
        socketOpen: false,
        hello: startingHello,
      },
      NOW,
    )
    expect(view.state).toBe('starting')
    expect(view.words).toBe('Starting HQ · 12 s')
    expect(view.dot).toBe('work')
    expect(view.actionsOff).toBe('You can answer once HQ has started.')
  })

  test('Lite connected gives Connected · HQ Lite', () => {
    const view = connectionState(
      {
        ...baseInputs,
        lite: true,
      },
      NOW,
    )
    expect(view.state).toBe('connected')
    expect(view.words).toBe('Connected · HQ Lite')
    expect(view.dot).toBe('off')
    expect(view.actionsOff).toBeNull()
  })

  test('the retry schedule gives 5, 10, 20, 20 s', () => {
    expect(retryDelaySec(0)).toBe(5)
    expect(retryDelaySec(1)).toBe(10)
    expect(retryDelaySec(2)).toBe(20)
    expect(retryDelaySec(3)).toBe(20)
    expect(retryDelaySec(4)).toBe(20)
  })

  test('within unreachable grace period with socket closed gives reconnecting', () => {
    const view = connectionState(
      {
        ...baseInputs,
        hello: null,
        helloAnsweredAt: NOW - (UNREACHABLE_GRACE_MS - 1000),
        socketOpen: false,
      },
      NOW,
    )
    expect(view.state).toBe('reconnecting')
  })

  test('proxy upstream gone triggers unreachable immediately', () => {
    const view = connectionState(
      {
        ...baseInputs,
        proxyUpstreamGone: true,
      },
      NOW,
    )
    expect(view.state).toBe('unreachable')
    expect(view.words).toBe("Your HQ can't be reached")
    expect(view.dot).toBe('warn')
    expect(view.actionsOff).toBe('These wait until your HQ can be reached.')
  })

  test('a page that has not asked HQ yet is not called unreachable', () => {
    const view = connectionState(
      { status401: false, online: true, helloAnsweredAt: null, lastHelloMs: null, hello: null, socketOpen: false, aiDown: false, proxyUpstreamGone: false, lite: false, inHub: false },
      Date.now(),
    )
    expect(view.state).not.toBe('unreachable')
  })
})
