// "A newer HQ is running": a new version of the app waits in the service worker.

import { useEffect, useState } from 'react'

export function useNewerHq(): boolean {
  const [waiting, setWaiting] = useState(false)
  useEffect(() => {
    if (!('serviceWorker' in navigator)) return
    let reg: ServiceWorkerRegistration | undefined
    const look = () => setWaiting(Boolean(reg?.waiting))
    navigator.serviceWorker.getRegistration().then((r) => {
      reg = r
      look()
      r?.addEventListener('updatefound', () => r.installing?.addEventListener('statechange', look))
    })
  }, [])
  return waiting
}

export async function reloadForNewer() {
  const reg = await navigator.serviceWorker.ready
  reg.waiting?.postMessage({ type: 'SKIP_WAITING' })
  window.location.reload()
}
