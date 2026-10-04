import { useCallback, useEffect, useState } from 'react'
import type { KeyboardEvent } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { searchNotes, type SearchHit } from '~/lib/vaultApi'

const SEARCH_DEBOUNCE_MS = 200

/** Debounced vault search with arrow, Enter and Escape handling for the search input. */
export function useVaultSearch(onClose: () => void) {
    const [query, setQuery] = useState('')
    const [results, setResults] = useState<SearchHit[]>([])
    const [isLoading, setIsLoading] = useState(false)
    const [selectedIndex, setSelectedIndex] = useState(0)
    const navigate = useNavigate()

    const reset = useCallback(() => {
        setQuery('')
        setResults([])
        setSelectedIndex(0)
    }, [])

    const openResult = useCallback((hit: SearchHit) => {
        reset()
        onClose()
        navigate({ to: '/vault/$', params: { _splat: hit.notePath } })
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
                const data = await searchNotes(q)
                if (!controller.signal.aborted) setResults(data.results)
            } catch {
                // Keep the previous results on a failed search.
            } finally {
                if (!controller.signal.aborted) setIsLoading(false)
            }
        }, SEARCH_DEBOUNCE_MS)
        return () => {
            clearTimeout(timer)
            controller.abort()
        }
    }, [query])

    return { query, setQuery, results, isLoading, selectedIndex, openResult, onKeyDown, reset }
}
