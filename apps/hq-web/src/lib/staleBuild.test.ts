import { describe, expect, test } from 'bun:test'
import { isStaleBuildError, reloadOnceForNewBuild } from './staleBuild'

function store(initial: Record<string, string> = {}) {
  const data = { ...initial }
  return {
    getItem: (k: string) => data[k] ?? null,
    setItem: (k: string, v: string) => {
      data[k] = v
    },
  }
}

describe('a page left open across an update', () => {
  test('each browser wording of a failed code file is recognised, and nothing else', () => {
    expect(isStaleBuildError(new TypeError('Importing a module script failed.'))).toBe(true)
    expect(isStaleBuildError(new TypeError('Failed to fetch dynamically imported module: x'))).toBe(true)
    expect(isStaleBuildError(new TypeError('error loading dynamically imported module'))).toBe(true)
    expect(isStaleBuildError(new Error('Cannot read properties of undefined'))).toBe(false)
    expect(isStaleBuildError(undefined)).toBe(false)
  })

  test('it reloads once, and a second failure right after shows the error', () => {
    const s = store()
    let reloads = 0
    const reload = () => {
      reloads += 1
    }
    expect(reloadOnceForNewBuild(s, 100_000, reload)).toBe(true)
    expect(reloadOnceForNewBuild(s, 101_000, reload)).toBe(false)
    expect(reloads).toBe(1)
    expect(reloadOnceForNewBuild(s, 200_000, reload)).toBe(true)
    expect(reloads).toBe(2)
  })

  test('with no storage it shows the error rather than risk a reload loop', () => {
    const broken = {
      getItem: () => {
        throw new Error('blocked')
      },
      setItem: () => undefined,
    }
    let reloads = 0
    expect(reloadOnceForNewBuild(broken, 1, () => (reloads += 1))).toBe(false)
    expect(reloads).toBe(0)
  })
})
