import { useState, useEffect, useRef, useCallback } from 'react'
import { useVaultSearch } from './useVaultSearch'

function HighlightedSnippet({ text, query }: { text: string; query: string }) {
  if (!query.trim()) return <>{text}</>
  const escaped = query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const regex = new RegExp(`(${escaped})`, 'gi')
  const parts = text.split(regex)
  return (
    <>
      {parts.map((part, i) =>
        regex.test(part) ? (
          <mark key={i} style={{ background: 'rgba(255,179,0,0.25)', color: 'var(--accent-amber)', borderRadius: 2 }}>{part}</mark>
        ) : (
          <span key={i}>{part}</span>
        )
      )}
    </>
  )
}

export function SearchOverlay() {
  const [isOpen, setIsOpen] = useState(false)
  const inputRef = useRef<HTMLInputElement>(null)
  const close = useCallback(() => setIsOpen(false), [])
  const { query, setQuery, results, isLoading, selectedIndex, openResult, onKeyDown, reset } = useVaultSearch(close)

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === '/') {
        e.preventDefault()
        setIsOpen((v) => !v)
      }
      if (e.key === 'Escape') setIsOpen(false)
    }
    const onOpen = () => setIsOpen(true)
    window.addEventListener('keydown', onKey)
    window.addEventListener('hq:open-search', onOpen)
    return () => {
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('hq:open-search', onOpen)
    }
  }, [])

  useEffect(() => {
    if (isOpen) {
      reset()
      inputRef.current?.focus()
    }
  }, [isOpen, reset])

  if (!isOpen) return null

  return (
    <>
      <div className="fixed inset-0 z-50 hq-scrim" onClick={() => setIsOpen(false)} />

      <div
        className="fixed z-50 w-full shadow-2xl overflow-hidden flex flex-col inset-0 pad-safe-top rounded-none md:inset-auto md:top-[10vh] md:left-1/2 md:-translate-x-1/2 md:max-w-2xl md:rounded-2xl md:max-h-[70vh] hq-modal"
      >
        <div className="flex items-center gap-3 px-4 py-3 border-b" style={{ borderColor: 'var(--border)' }}>
          <span className="text-base" style={{ color: 'var(--text-dim)' }}>⌕</span>
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder="Search vault..."
            className="flex-1 bg-transparent outline-none "
            style={{ color: 'var(--text-primary)', fontSize: '16px', caretColor: 'var(--accent-green)' }}
            enterKeyHint="go"
          />
          {isLoading && (
            <div className="w-4 h-4 rounded-full border-2 border-t-transparent animate-spin" style={{ borderColor: 'var(--accent-amber)', borderTopColor: 'transparent' }} />
          )}
          <button onClick={() => setIsOpen(false)} style={{ color: 'var(--text-dim)' }}>
            <span className="text-sm ">✕</span>
          </button>
        </div>

        <div className="flex-1 overflow-y-auto">
          {query && !isLoading && results.length === 0 && (
            <div className="px-6 py-12 text-center text-sm " style={{ color: 'var(--text-dim)' }}>
              No vault results for "{query}"
            </div>
          )}

          {results.map((hit, idx) => (
            <button
              key={hit.notePath}
              onClick={() => openResult(hit)}
              className="w-full text-left px-4 py-3 transition-all"
              style={{
                borderBottom: '1px solid rgba(255,255,255,0.04)',
                background: idx === selectedIndex ? 'rgba(167,139,250,0.06)' : 'transparent',
                borderLeft: idx === selectedIndex ? '2px solid var(--accent-violet)' : '2px solid transparent',
              }}
            >
              <div className="flex items-center justify-between gap-3">
                <span className="text-sm font-bold truncate" style={{ color: 'var(--text-primary)' }}>
                  <HighlightedSnippet text={hit.title} query={query} />
                </span>
                <span className="text-[11px] flex-shrink-0" style={{ color: 'var(--text-dim)' }}>
                  {hit.notebook}
                </span>
              </div>
              {hit.snippet && (
                <p className="text-xs line-clamp-2 leading-relaxed mt-1" style={{ color: 'var(--text-dim)' }}>
                  <HighlightedSnippet text={hit.snippet} query={query} />
                </p>
              )}
              <div className="text-[9px] mt-1.5 truncate" style={{ color: 'var(--accent-blue)', opacity: 0.55 }}>
                {hit.notePath}
              </div>
            </button>
          ))}
        </div>

        {results.length > 0 && (
          <div className="px-4 py-2 flex items-center justify-between text-[11px] " style={{ color: 'var(--text-dim)', borderTop: '1px solid rgba(255,255,255,0.04)' }}>
            <span>{results.length} vault results</span>
            <div className="flex items-center gap-3">
              <span><kbd className="px-1 rounded" style={{ background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.08)' }}>↑↓</kbd> navigate</span>
              <span><kbd className="px-1 rounded" style={{ background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.08)' }}>↵</kbd> open</span>
            </div>
          </div>
        )}
      </div>
    </>
  )
}
