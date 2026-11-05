// Whether this page is HQ's window inside HQ Hub: its proxy marks every response `x-hq-window: hub`
// (B11). Read once, from /api/hello, which is open without a sign-in.

import { useEffect, useState } from 'react'

export const HUB_HEADER = { name: 'x-hq-window', value: 'hub' }

export function isHubResponse(headers: Pick<Headers, 'get'>): boolean {
  return headers.get(HUB_HEADER.name) === HUB_HEADER.value
}

let known: boolean | null = null

export function useInHub(): boolean {
  const [hub, setHub] = useState(known ?? false)
  useEffect(() => {
    if (known !== null) return
    fetch('/api/hello', { cache: 'no-store' })
      .then((res) => {
        known = isHubResponse(res.headers)
        setHub(known)
      })
      .catch(() => undefined)
  }, [])
  return hub
}
