import { Link } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import { deviceTimeZone, readClock, useClockZone } from '~/lib/clock'

const TICK_MS = 1_000

/**
 * Current time in the zone chosen under Settings > Clock (this device's zone by default).
 * Renders a placeholder until mounted, so the prerendered shell never carries a stale time.
 */
export function HeaderClock() {
  const pinned = useClockZone()
  const [now, setNow] = useState<Date | null>(null)

  useEffect(() => {
    setNow(new Date())
    // Checked every second so the minute flips on time and a phone waking from sleep catches up at once.
    const id = window.setInterval(() => setNow(new Date()), TICK_MS)
    return () => window.clearInterval(id)
  }, [])

  const zone = pinned || deviceTimeZone()
  const reading = now ? readClock(now, zone) : null

  return (
    <Link
      to="/settings"
      hash="clock"
      title={reading ? `${reading.full}. Change the zone in Settings.` : 'Clock'}
      aria-label={reading ? `Time: ${reading.full}` : 'Clock'}
      className="flex items-baseline gap-1 px-2 py-0.5 rounded-md border border-white/10 font-mono tabular-nums whitespace-nowrap flex-shrink-0 hover:bg-white/5 transition-colors"
    >
      <span suppressHydrationWarning className="text-[13px] md:text-sm font-semibold text-neutral-100">
        {reading?.time ?? '--:--'}
      </span>
      <span suppressHydrationWarning className="hidden min-[360px]:inline text-[9px] uppercase text-neutral-400">
        {reading?.abbrev ?? ''}
      </span>
    </Link>
  )
}
