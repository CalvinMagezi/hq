export type SectionState<T> =
  | { kind: 'loading' }
  | { kind: 'error'; message: string }
  | { kind: 'empty' }
  | { kind: 'ready'; items: T[] }

interface QueryLike<T> {
  data: T[] | undefined
  isPending: boolean
  isError: boolean
  error: unknown
}

/** Saved data wins over an error, so a failed refresh never hides what is already on screen. */
export function sectionState<T>(q: QueryLike<T>): SectionState<T> {
  if (q.data && q.data.length > 0) return { kind: 'ready', items: q.data }
  if (q.isError) return { kind: 'error', message: q.error instanceof Error ? q.error.message : 'Request failed' }
  if (q.isPending) return { kind: 'loading' }
  return { kind: 'empty' }
}
