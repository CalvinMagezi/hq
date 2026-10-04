import { useEffect, useState } from 'react'

interface BeforeInstallPromptEvent extends Event {
  prompt: () => Promise<void>
  userChoice: Promise<{ outcome: 'accepted' | 'dismissed' }>
}

const DISMISSED_KEY = 'hq-install-dismissed'

export function InstallPrompt() {
  const [deferredPrompt, setDeferredPrompt] = useState<BeforeInstallPromptEvent | null>(null)
  const [showIOSHint, setShowIOSHint] = useState(false)
  const [visible, setVisible] = useState(false)

  useEffect(() => {
    if (typeof window === 'undefined') return
    if (localStorage.getItem(DISMISSED_KEY)) return

    // Check if already installed
    if (window.matchMedia('(display-mode: standalone)').matches) return

    // iOS Safari — no beforeinstallprompt, show manual hint
    const isIOS = /iphone|ipad|ipod/i.test(navigator.userAgent) && !(window as any).MSStream
    const isSafari = /^((?!chrome|android).)*safari/i.test(navigator.userAgent)
    if (isIOS && isSafari) {
      setShowIOSHint(true)
      setVisible(true)
      return
    }

    const handler = (e: Event) => {
      e.preventDefault()
      setDeferredPrompt(e as BeforeInstallPromptEvent)
      setVisible(true)
    }
    window.addEventListener('beforeinstallprompt', handler)
    return () => window.removeEventListener('beforeinstallprompt', handler)
  }, [])

  const handleInstall = async () => {
    if (!deferredPrompt) return
    await deferredPrompt.prompt()
    const { outcome } = await deferredPrompt.userChoice
    if (outcome === 'accepted') {
      localStorage.setItem(DISMISSED_KEY, '1')
    }
    setDeferredPrompt(null)
    setVisible(false)
  }

  const handleDismiss = () => {
    localStorage.setItem(DISMISSED_KEY, '1')
    setVisible(false)
  }

  if (!visible) return null

  return (
    <div
      className="flex items-center justify-between px-4 py-2 text-sm gap-3"
      style={{ background: 'rgba(68,136,255,0.08)', borderBottom: '1px solid rgba(68,136,255,0.2)' }}
    >
      <span style={{ color: 'var(--text-secondary, var(--text-dim))' }} className="text-xs font-mono">
        {showIOSHint
          ? '📲 Install HQ: tap the Share button then "Add to Home Screen"'
          : '📲 Install HQ to your home screen for faster access'}
      </span>
      <div className="flex items-center gap-2 shrink-0">
        {!showIOSHint && (
          <button
            onClick={handleInstall}
            className="px-3 py-1.5 rounded text-xs font-mono font-bold"
            style={{ background: 'var(--accent-blue)', color: '#fff', minHeight: 36 }}
          >
            Install
          </button>
        )}
        <button
          onClick={handleDismiss}
          className="px-2 py-1 rounded text-xs font-mono"
          style={{ color: 'var(--text-dim)', minHeight: 36 }}
        >
          ✕
        </button>
      </div>
    </div>
  )
}
