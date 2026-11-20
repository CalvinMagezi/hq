import { useCallback, useState } from 'react'

export type Density = 'comfortable' | 'compact'
export const DENSITY_KEY = 'hq.suite.density'

/** The saved choice; a browser that refuses storage gives comfortable. */
export function readDensity(): Density {
  try {
    return localStorage.getItem(DENSITY_KEY) === 'compact' ? 'compact' : 'comfortable'
  } catch {
    return 'comfortable'
  }
}

export function saveDensity(d: Density): void {
  try {
    localStorage.setItem(DENSITY_KEY, d)
  } catch {
    // Blocked storage: the choice lasts until the page closes.
  }
}

/** Grid density, remembered on this device only. Compact applies to a mouse or pen; a finger always gets full-size cells. */
export function useDensity(): [Density, () => void] {
  const [density, set] = useState<Density>(readDensity)
  const toggle = useCallback(() => {
    set((d) => {
      const next = d === 'compact' ? 'comfortable' : 'compact'
      saveDensity(next)
      return next
    })
  }, [])
  return [density, toggle]
}
