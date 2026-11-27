import { useQueryClient } from '@tanstack/react-query'
import { useCallback, useState } from 'react'
import { vaultHealthQuery, vaultKeys } from './queries'
import { toastAlert, toastNotice } from './toast'
import type { VaultHealth } from './vaultHealthApi'

/** What a change gives back: the report as it is now, and a line for the person (or none). */
export interface HealthChange {
  health: VaultHealth
  message: string
}

/**
 * Runs one repair, undo or ignore for the notes check and shows its result: the new report
 * replaces the old, open notes and lists refetch (a fix changes notes), and a failure says so in
 * words. `busyKey` names what is running, so a button can say it is working.
 */
export function useHealthChange() {
  const queryClient = useQueryClient()
  const [busyKey, setBusyKey] = useState<string | null>(null)
  const change = useCallback(
    async (key: string, work: () => Promise<HealthChange>, failed: string) => {
      setBusyKey(key)
      try {
        const { health, message } = await work()
        queryClient.setQueryData(vaultHealthQuery.queryKey, health)
        void queryClient.invalidateQueries({
          queryKey: vaultKeys.all,
          predicate: (query) => query.queryKey[1] !== 'health',
        })
        if (message) toastNotice(message)
      } catch {
        toastAlert(failed)
      } finally {
        setBusyKey(null)
      }
    },
    [queryClient],
  )
  return { busyKey, change }
}
