/** The second note's title when someone keeps both versions: "Plan, your version". */
export function keepBothTitle(path: string, suffix: string): string {
  const base = (path.split('/').pop() ?? '').replace(/\.(md|markdown|txt)$/i, '')
  return `${base || 'Note'}, ${suffix}`
}
