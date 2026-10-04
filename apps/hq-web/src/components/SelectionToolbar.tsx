import { useEffect, useState } from 'react'

export function SelectionToolbar({ containerRef, onSendAction }: { containerRef: React.RefObject<HTMLElement | null>, onSendAction: (text: string) => void }) {
    const [selection, setSelection] = useState<{ text: string; rect: DOMRect } | null>(null)

    useEffect(() => {
        const handleMouseUp = () => {
            const activeSelection = window.getSelection()
            if (!activeSelection || activeSelection.isCollapsed) {
                setSelection(null)
                return
            }

            const text = activeSelection.toString().trim()
            if (!text || !containerRef.current) {
                setSelection(null)
                return
            }

            // Ensure selection is inside the container
            if (!containerRef.current.contains(activeSelection.anchorNode)) {
                setSelection(null)
                return
            }

            const range = activeSelection.getRangeAt(0)
            const rect = range.getBoundingClientRect()

            // Don't show if hidden or off-screen
            if (rect.width === 0 || rect.height === 0) {
                setSelection(null)
                return
            }

            setSelection({ text, rect })
        }

        document.addEventListener('mouseup', handleMouseUp)
        // Handle double-click selections as well
        document.addEventListener('dblclick', handleMouseUp)

        // Clear selection when clicking outside
        const handleMouseDown = (e: MouseEvent) => {
            if (selection && activeSelectionOutsideToolbar(e)) {
                setSelection(null)
            }
        }
        document.addEventListener('mousedown', handleMouseDown)

        return () => {
            document.removeEventListener('mouseup', handleMouseUp)
            document.removeEventListener('dblclick', handleMouseUp)
            document.removeEventListener('mousedown', handleMouseDown)
        }
    }, [containerRef, selection])

    function activeSelectionOutsideToolbar(e: MouseEvent) {
        const el = document.getElementById('hq-selection-toolbar')
        return el && !el.contains(e.target as Node)
    }

    if (!selection) return null

    return (
        <div
            id="hq-selection-toolbar"
            className="fixed z-50 flex items-center gap-1 rounded-lg shadow-xl px-1.5 py-1.5 transform -translate-x-1/2 -mt-12 transition-all"
            style={{
                background: 'var(--bg-elevated)',
                border: '1px solid var(--border)',
                left: selection.rect.left + selection.rect.width / 2,
                top: selection.rect.top,
            }}
        >
            <button
                onClick={() => {
                    onSendAction(selection.text)
                    window.getSelection()?.removeAllRanges()
                    setSelection(null)
                }}
                className="text-xs font-mono font-bold px-3 py-1.5 rounded transition-colors flex items-center gap-1.5 hover:bg-white/10"
                style={{ color: 'var(--text-primary)' }}
            >
                <span className="opacity-70">💬</span> Send to Chat
            </button>
        </div>
    )
}
