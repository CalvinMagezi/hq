import { useCallback, useEffect, useState } from 'react'
import { parseSidebarLayout } from '~/lib/sidebarLayout'

const STORAGE_KEY = 'hq.workbench.sidebar'

/**
 * Whether the Helpers list is folded away, remembered on this device.
 * Read in an effect, never during render, so the prerender and first client paint agree.
 */
export function useSidebarLayout() {
  const [collapsed, setCollapsed] = useState(false)

  useEffect(() => {
    try {
      setCollapsed(parseSidebarLayout(window.localStorage.getItem(STORAGE_KEY)).collapsed)
    } catch {
      // Storage can be blocked; the list simply starts open.
    }
  }, [])

  const toggle = useCallback(() => {
    setCollapsed((was) => {
      try {
        window.localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...parseSidebarLayout(window.localStorage.getItem(STORAGE_KEY)), collapsed: !was }))
      } catch {
        // Not remembering is fine.
      }
      return !was
    })
  }, [])

  return { collapsed, toggle }
}
