import { expect, test } from 'bun:test'
import { visibleTimer } from './visibleTimer'

function fakeDoc(state: string) {
  const listeners = new Set<() => void>()
  return {
    visibilityState: state,
    addEventListener: (_: 'visibilitychange', cb: () => void) => void listeners.add(cb),
    removeEventListener: (_: 'visibilitychange', cb: () => void) => void listeners.delete(cb),
    set(next: string) {
      this.visibilityState = next
      for (const l of listeners) l()
    },
  }
}

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms))

test('check 3: a hidden tab runs nothing, and one run happens when it is shown', async () => {
  const doc = fakeDoc('visible')
  let runs = 0
  const stop = visibleTimer(() => runs++, 10, doc)
  await wait(35)
  expect(runs).toBeGreaterThanOrEqual(2)
  doc.set('hidden')
  const atHide = runs
  await wait(60)
  expect(runs).toBe(atHide)
  doc.set('visible')
  expect(runs).toBe(atHide + 1)
  stop()
  const atStop = runs
  await wait(30)
  expect(runs).toBe(atStop)
})
