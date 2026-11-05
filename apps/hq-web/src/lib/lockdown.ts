import { t } from '~/strings'
import type { HostLockdown, LockdownResult } from './sessionsApi'
import { computerName } from './workbench'

const clock = (epochSeconds: number) => new Date(epochSeconds * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })

/** The fact under the lock banner: who stopped the helpers on that computer, and when. */
export function lockFact(host: string, lock: HostLockdown): string {
  const args = { computer: computerName(host), time: clock(lock.at) }
  if (lock.source === 'breaker') return t('helpers_locked_fact_breaker', args)
  if (lock.source === 'operator') return t('helpers_locked_fact_owner', args)
  return t('helpers_locked_fact_other', args)
}

/** Computers that did not answer, so their helpers were not stopped; never reported as locked. */
export function unreachableHosts(result: LockdownResult): string[] {
  return result.hosts.filter((h) => !h.locked).map((h) => h.host)
}
