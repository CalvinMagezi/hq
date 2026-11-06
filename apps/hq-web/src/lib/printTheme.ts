import { useEffect } from 'react'

const THEME_ATTR = 'data-theme'
const PRINT_THEME = 'light'

/** Paper is white and printers drop backgrounds, so a printout takes the design's light values. */
export function usePrintInLight(): void {
  useEffect(() => {
    const root = document.documentElement
    const light = () => root.setAttribute(THEME_ATTR, PRINT_THEME)
    const back = () => root.removeAttribute(THEME_ATTR)
    window.addEventListener('beforeprint', light)
    window.addEventListener('afterprint', back)
    return () => {
      window.removeEventListener('beforeprint', light)
      window.removeEventListener('afterprint', back)
    }
  }, [])
}
