import { afterEach, describe, expect, test } from 'bun:test'
import { isSharedComputer, setSharedComputer, tokenStorage, watchIdle } from './sharedComputer'

const g = globalThis as unknown as Record<string, unknown>
function storage() {
  const m = new Map<string, string>()
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), removeItem: (k: string) => void m.delete(k) } as unknown as Storage
}

afterEach(() => {
  delete g.sessionStorage
  delete g.localStorage
})

describe('a shared computer', () => {
  test('keeps the sign-in in this tab only', () => {
    g.sessionStorage = storage()
    g.localStorage = storage()
    expect(tokenStorage()).toBe(g.localStorage as Storage)
    setSharedComputer(true)
    expect(isSharedComputer()).toBe(true)
    expect(tokenStorage()).toBe(g.sessionStorage as Storage)
    setSharedComputer(false)
    expect(isSharedComputer()).toBe(false)
  })

  test('signs out after the idle time, and input starts it again', async () => {
    const handlers: Record<string, () => void> = {}
    const target = { addEventListener: (e: string, h: () => void) => void (handlers[e] = h), removeEventListener: () => {} }
    let idle = 0
    const stop = watchIdle(() => idle++, target as unknown as Window, 30)
    await new Promise((r) => setTimeout(r, 20))
    handlers.keydown()
    await new Promise((r) => setTimeout(r, 20))
    expect(idle).toBe(0)
    await new Promise((r) => setTimeout(r, 25))
    expect(idle).toBe(1)
    stop()
  })
})
