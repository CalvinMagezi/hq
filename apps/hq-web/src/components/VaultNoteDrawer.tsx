import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { X, FileText, AlertCircle, Loader2, Copy, Check, Folder, ArrowRight } from 'lucide-react'
import { useVaultNoteStore } from '~/store/vaultNoteStore'
import { fetchNoteForDrawer, type NoteDetailResponse } from '~/lib/vaultApi'
import { MarkdownViewer } from './MarkdownViewer'

export function VaultNoteDrawer() {
  const { open, notePath, closeNote } = useVaultNoteStore()
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [data, setData] = useState<NoteDetailResponse | null>(null)
  const [copied, setCopied] = useState(false)
  const closeButtonRef = useRef<HTMLButtonElement>(null)
  const drawerRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open || !notePath) {
      setData(null)
      setError(null)
      setLoading(false)
      return
    }

    let active = true
    setLoading(true)
    setError(null)

    fetchNoteForDrawer(notePath)
      .then((res) => {
        if (!active) return
        setData(res)
        setLoading(false)
      })
      .catch((err) => {
        if (!active) return
        setError(err instanceof Error ? err.message : String(err))
        setLoading(false)
      })

    return () => {
      active = false
    }
  }, [open, notePath])

  // Focus management, Escape key, and a Tab focus trap so keyboard focus
  // never leaves the drawer while it's the active dialog.
  useEffect(() => {
    if (!open) return

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation()
        closeNote()
        return
      }
      if (e.key !== 'Tab' || !drawerRef.current) return
      const focusable = drawerRef.current.querySelectorAll<HTMLElement>(
        'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
      )
      if (focusable.length === 0) return
      const first = focusable[0]
      const last = focusable[focusable.length - 1]
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault()
        last.focus()
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault()
        first.focus()
      }
    }

    window.addEventListener('keydown', handleKeyDown)
    // Focus close button on open
    setTimeout(() => {
      closeButtonRef.current?.focus()
    }, 50)

    return () => {
      window.removeEventListener('keydown', handleKeyDown)
    }
  }, [open, closeNote])

  if (!open || !notePath) return null

  const handleCopyPath = () => {
    if (!notePath) return
    void navigator.clipboard.writeText(notePath)
    setCopied(true)
    setTimeout(() => setCopied(false), 1500)
  }

  const title = data?.path ? data.path.split('/').pop()?.replace(/\.md$/, '') || data.path : notePath

  const content = (
    <div
      className="fixed inset-0 z-[60] flex justify-end bg-black/60 backdrop-blur-sm transition-all duration-300"
      onClick={(e) => {
        if (e.target === e.currentTarget) closeNote()
      }}
    >
      <div className="flex-1" onClick={closeNote} />

      <div
        ref={drawerRef}
        role="dialog"
        aria-modal="true"
        aria-label={`Vault note: ${title}`}
        className="w-full max-w-2xl h-full flex flex-col glass-heavy border-l shadow-2xl overflow-hidden animate-in slide-in-from-right duration-200 pad-safe-top pb-[var(--safe-bottom)]"
        style={{ background: 'var(--bg-card, #111418)', borderColor: 'rgba(255,255,255,0.1)' }}
      >
        {/* Header */}
        <div
          className="flex items-center justify-between px-6 py-4 border-b flex-shrink-0"
          style={{ borderColor: 'rgba(255,255,255,0.08)' }}
        >
          <div className="min-w-0 pr-3">
            <div className="flex items-center gap-2">
              {data?.isDir ? (
                <Folder className="w-4 h-4 text-amber-400 shrink-0" />
              ) : (
                <FileText className="w-4 h-4 text-emerald-400 shrink-0" />
              )}
              <h2 className="text-sm font-semibold text-white truncate">{title}</h2>
            </div>
            <div className="flex items-center gap-2 mt-0.5">
              <span className="text-[11px] font-mono text-neutral-400 truncate max-w-md">{notePath}</span>
              <button
                type="button"
                onClick={handleCopyPath}
                title="Copy relative path"
                className="p-1 rounded text-neutral-400 hover:text-white hover:bg-white/10 transition-colors"
                aria-label="Copy relative path"
              >
                {copied ? <Check className="w-3 h-3 text-emerald-400" /> : <Copy className="w-3 h-3" />}
              </button>
            </div>
          </div>

          <button
            ref={closeButtonRef}
            type="button"
            onClick={closeNote}
            className="p-2 rounded-xl text-neutral-400 hover:text-white hover:bg-white/10 transition-colors"
            aria-label="Close note drawer"
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        {/* Content Area */}
        <div className="flex-1 overflow-y-auto px-6 py-5">
          {loading && (
            <div className="flex flex-col items-center justify-center py-20 text-neutral-400" role="status">
              <Loader2 className="w-6 h-6 animate-spin text-emerald-400 mb-2" />
              <span className="text-xs font-mono">Loading note...</span>
            </div>
          )}

          {!loading && error && (
            <div className="p-4 rounded-xl border border-rose-500/20 bg-rose-500/10 text-rose-300">
              <div className="flex items-center gap-2 mb-2 font-semibold text-sm">
                <AlertCircle className="w-4 h-4" />
                Error loading note
              </div>
              <p className="text-xs font-mono text-rose-300/80 mb-3">{error}</p>
              <button
                type="button"
                onClick={() => {
                  setLoading(true)
                  setError(null)
                  fetchNoteForDrawer(notePath)
                    .then((res) => {
                      setData(res)
                      setLoading(false)
                    })
                    .catch((err) => {
                      setError(err instanceof Error ? err.message : String(err))
                      setLoading(false)
                    })
                }}
                className="px-3 py-1.5 rounded-lg text-xs font-mono font-medium border border-rose-500/30 hover:bg-rose-500/20 text-white"
              >
                Retry
              </button>
            </div>
          )}

          {!loading && !error && data?.notFound && (
            <div className="py-16 text-center text-neutral-400 space-y-3">
              <div className="w-12 h-12 rounded-2xl bg-neutral-800/80 border border-white/10 flex items-center justify-center mx-auto text-neutral-500">
                <FileText className="w-6 h-6" />
              </div>
              <div className="font-semibold text-neutral-200">Note Not Found</div>
              <p className="text-xs text-neutral-400 max-w-sm mx-auto">
                <span className="font-mono text-neutral-300">{notePath}</span> does not exist in the vault or may have been moved.
              </p>
              <button
                type="button"
                onClick={closeNote}
                className="px-4 py-2 rounded-xl text-xs font-mono font-medium bg-white/5 hover:bg-white/10 text-neutral-200 border border-white/10 transition-colors"
              >
                Dismiss
              </button>
            </div>
          )}

          {!loading && !error && data && !data.notFound && (
            <div>
              {data.isDir ? (
                <div className="space-y-2">
                  <div className="text-xs font-mono uppercase text-neutral-500 mb-3">Directory Contents</div>
                  {(!data.dirEntries || data.dirEntries.length === 0) && (
                    <div className="text-xs text-neutral-500 italic">Empty directory</div>
                  )}
                  {data.dirEntries?.map((entry) => (
                    <button
                      key={entry.path}
                      type="button"
                      onClick={() => useVaultNoteStore.getState().openNote(entry.path)}
                      className="w-full flex items-center justify-between p-3 rounded-xl bg-white/5 hover:bg-white/10 border border-white/5 text-left text-xs font-mono transition-colors group"
                    >
                      <div className="flex items-center gap-2 truncate">
                        {entry.isDir ? (
                          <Folder className="w-4 h-4 text-amber-400 shrink-0" />
                        ) : (
                          <FileText className="w-4 h-4 text-emerald-400 shrink-0" />
                        )}
                        <span className="text-neutral-200 truncate">{entry.name}</span>
                      </div>
                      <ArrowRight className="w-3.5 h-3.5 text-neutral-500 group-hover:text-white transition-colors shrink-0" />
                    </button>
                  ))}
                </div>
              ) : (
                <div className="prose prose-invert max-w-none text-sm leading-relaxed">
                  <MarkdownViewer content={data.content || '*Empty note*'} bare={false} />
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  )

  return typeof document !== 'undefined' ? createPortal(content, document.body) : null
}
