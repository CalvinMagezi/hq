// The few choices that HQ itself keeps for the owner (`/api/prefs`), not this browser: what HQ does on its own
// (naming chats) and which helper requests reach the owner.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { hqJson } from './hqAuth'

export type AskLevel = 'nothing' | 'loses_work' | 'everything'
export const ASK_LEVELS: AskLevel[] = ['nothing', 'loses_work', 'everything']

export interface Prefs {
  'chat.auto_title': boolean
  'memory.learning': boolean
  'helpers.ask_level': AskLevel
}

const KEY = ['prefs']

export const fetchPrefs = () => hqJson<Prefs>('/api/prefs')

export function setPref<K extends keyof Prefs>(key: K, value: Prefs[K]): Promise<Prefs> {
  return hqJson<Prefs>('/api/prefs', 'PUT', { key, value })
}

/** The owner's choices. Undefined until HQ answers, or when an older HQ has no such route. */
export function usePrefs() {
  const client = useQueryClient()
  const query = useQuery({ queryKey: KEY, queryFn: fetchPrefs, staleTime: 30_000, retry: false })
  const change = useMutation({
    mutationFn: ({ key, value }: { key: keyof Prefs; value: Prefs[keyof Prefs] }) => setPref(key, value as never),
    // The choice shows at once; if HQ refuses, what it still holds comes back.
    onMutate: async ({ key, value }) => {
      await client.cancelQueries({ queryKey: KEY })
      const before = client.getQueryData<Prefs>(KEY)
      if (before) client.setQueryData<Prefs>(KEY, { ...before, [key]: value })
      return { before }
    },
    onError: (_e, _v, ctx) => {
      if (ctx?.before) client.setQueryData(KEY, ctx.before)
    },
    onSuccess: (next) => client.setQueryData(KEY, next),
  })
  return { prefs: query.data, loading: query.isPending, failed: query.isError || change.isError, set: change.mutate }
}
