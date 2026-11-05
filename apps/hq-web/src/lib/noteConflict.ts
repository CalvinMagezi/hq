/** The second note's title when someone keeps both versions: "Plan, your version". */
export function keepBothTitle(path: string, suffix: string): string {
  const base = (path.split('/').pop() ?? '').replace(/\.(md|markdown|txt)$/i, '')
  return `${base || 'Note'}, ${suffix}`
}

export type ConflictWhere = 'hq' | 'helper' | 'unknown'

/** Where the newer version came from, as far as HQ recorded it (`by` is the writer's source kind). */
export function conflictWhere(by?: string): ConflictWhere {
  if (by === 'agent') return 'helper'
  if (by === 'generated' || by === 'system') return 'hq'
  return 'unknown'
}

/** How many lines differ between the two texts, counting lines only one side has. */
export function differingLines(mine: string, theirs: string): number {
  const a = mine.split('\n')
  const b = theirs.split('\n')
  let n = 0
  for (let i = 0; i < Math.max(a.length, b.length); i++) if (a[i] !== b[i]) n++
  return n
}

export function stripFrontmatter(text: string): string {
  return text.replace(/^---[\s\S]*?---\n?/, '')
}
