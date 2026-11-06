// Whether this page is HQ's window inside HQ Hub: its proxy marks every response `x-hq-window: hub`
// (B11), and `x-hq-hub: phone` when the HQ it shows is a paired phone's. Read once, from
// /api/hello, which is open without a sign-in.

import { useEffect, useState } from 'react'

export const HUB_HEADER = { name: 'x-hq-window', value: 'hub' }
export const HUB_KIND_HEADER = { name: 'x-hq-hub', phone: 'phone' }

export function isHubResponse(headers: Pick<Headers, 'get'>): boolean {
  return headers.get(HUB_HEADER.name) === HUB_HEADER.value
}

/** Whether the window shows HQ on a paired phone; only HQ Hub's proxy says so, never HQ itself. */
export function isPhoneHubResponse(headers: Pick<Headers, 'get'>): boolean {
  return isHubResponse(headers) && headers.get(HUB_KIND_HEADER.name) === HUB_KIND_HEADER.phone
}

interface Window {
  hub: boolean
  phone: boolean
}

const OUTSIDE: Window = { hub: false, phone: false }
let known: Window | null = null
let asking: Promise<Window> | null = null

function ask(): Promise<Window> {
  asking ??= fetch('/api/hello', { cache: 'no-store' })
    .then((res) => {
      known = { hub: isHubResponse(res.headers), phone: isPhoneHubResponse(res.headers) }
      return known
    })
    .catch(() => {
      asking = null
      return OUTSIDE
    })
  return asking
}

/** Whether the page already knows it is HQ's window; false until the first hello answers. */
export function knownInHub(): boolean {
  return known?.hub === true
}

function useWindow(): Window {
  const [win, setWin] = useState(known ?? OUTSIDE)
  useEffect(() => {
    if (known !== null) return
    let live = true
    void ask().then((w) => live && setWin(w))
    return () => {
      live = false
    }
  }, [])
  return win
}

export function useInHub(): boolean {
  return useWindow().hub
}

/** The HQ window showing HQ on your phone, so the page can say whose HQ it is. */
export function useHubPhone(): boolean {
  return useWindow().phone
}
