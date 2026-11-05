import { useCallback, useEffect, useState } from 'react'
import type { KeyboardEvent } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { searchNotes, type SearchHit } from '~/lib/vaultApi'
import { searchDebounceMs } from '~/lib/dataUsage'
import { SEARCH_DEBOUNCE_MS } from '~/lib/search'

/** Debounced vault search with arrow, Enter and Escape handling for the search input. */
export function useVaultSearch(onClose: () => void, folder?: string | null) {
    const [query, setQuery] = useState('')
    const [results, setResults] = useState<SearchHit[]>([])
    const [isLoading, setIsLoading] = useState(false)
    const [failed, setFailed] = useState(false)
    const [selectedIndex, setSelectedIndex] = useState(0)
    const navigate = useNavigate()
    const [retry, setRetry] = useState(0)
    // The device coming back online runs the same search again by itself; the overlay calls `again` when HQ is back.
    const again = useCallback(() => setRetry((n) => n + 1), [])
    useEffect(() => {
        window.addEventListener('online', again)
        return () => window.removeEventListener('online', again)
    }, [again])

    const reset = useCallback(() => {
        setQuery('')
        setResults([])
        setSelectedIndex(0)
    }, [])

    const openResult = useCallback((hit: SearchHit) => {
        reset()
        onClose()
        navigate({ to: '/notes/$', params: { _splat: hit.notePath } })
    }, [reset, onClose, navigate])

    const onKeyDown = useCallback((e: KeyboardEvent) => {
        if (e.key === 'ArrowDown') {
            e.preventDefault()
            setSelectedIndex((i) => (i + 1) % Math.max(results.length, 1))
        } else if (e.key === 'ArrowUp') {
            e.preventDefault()
            setSelectedIndex((i) => (i - 1 + results.length) % Math.max(results.length, 1))
        } else if (e.key === 'Enter' && results[selectedIndex]) {
            e.preventDefault()
            openResult(results[selectedIndex])
        } else if (e.key === 'Escape') {
            reset()
            onClose()
        }
    }, [results, selectedIndex, openResult, reset, onClose])

    useEffect(() => {
        const q = query.trim()
        if (!q) {
            setResults([])
            setIsLoading(false)
            return
        }
        const controller = new AbortController()
        setIsLoading(true)
        setSelectedIndex(0)
        const timer = setTimeout(async () => {
            try {
                const data = await searchNotes(q, folder)
                if (!controller.signal.aborted) {
                    setResults(data.results)
                    setFailed(false)
                }
            } catch {
                if (!controller.signal.aborted) setFailed(true)
            } finally {
                if (!controller.signal.aborted) setIsLoading(false)
            }
        }, searchDebounceMs(SEARCH_DEBOUNCE_MS))
        return () => {
            clearTimeout(timer)
            controller.abort()
        }
    }, [query, folder, retry])

    return { query, setQuery, results, isLoading, failed, again, selectedIndex, openResult, onKeyDown, reset }
}
