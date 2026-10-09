import React, { useEffect, useRef } from 'react'
import type { NoteExportFormat, NoteExportGroup, NoteExportOption } from '~/lib/noteExport'

export interface ExportMenuState {
    x: number
    y: number
}

const MENU_W = 240
const ITEM_H = 40

const GROUP_TITLES: Record<NoteExportGroup, string> = {
    document: 'Document',
    image: 'Image',
    data: 'Tables in this note',
}

/**
 * Floating "Export as" menu for a note. Rendered when `menu` is set and anchored near the button that
 * opened it, clamped to the viewport so it stays usable on phones. Like `CopyPathMenu`, it closes on an
 * outside tap, a scroll or Escape. `busy` is the format being exported; every item is disabled while one runs.
 */
export function ExportMenu({
    menu,
    options,
    busy,
    onPick,
    onClose,
}: {
    menu: ExportMenuState | null
    options: readonly NoteExportOption[]
    busy: NoteExportFormat | null
    onPick: (format: NoteExportFormat) => void
    onClose: () => void
}) {
    const ref = useRef<HTMLDivElement | null>(null)

    useEffect(() => {
        if (!menu) return
        const dismiss = (e: Event) => {
            if (ref.current && e.target instanceof Node && ref.current.contains(e.target)) return
            onClose()
        }
        const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
        // Delay attach so the tap that opened the menu doesn't instantly close it.
        const t = setTimeout(() => {
            window.addEventListener('pointerdown', dismiss)
            window.addEventListener('scroll', dismiss, true)
            window.addEventListener('keydown', onKey)
        }, 50)
        // Keyboard users land on the first choice.
        ref.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus()
        return () => {
            clearTimeout(t)
            window.removeEventListener('pointerdown', dismiss)
            window.removeEventListener('scroll', dismiss, true)
            window.removeEventListener('keydown', onKey)
        }
    }, [menu, onClose])

    if (!menu) return null

    const groups = (['document', 'image', 'data'] as const)
        .map((group) => ({ group, items: options.filter((o) => o.group === group) }))
        .filter((g) => g.items.length > 0)
    const height = groups.length * 26 + options.length * ITEM_H
    const left = Math.max(8, Math.min(menu.x, window.innerWidth - MENU_W - 8))
    const top = Math.max(8, Math.min(menu.y, window.innerHeight - height - 8))

    const itemStyle: React.CSSProperties = {
        display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, width: '100%',
        padding: '0 14px', background: 'none', border: 'none', textAlign: 'left',
        fontFamily: 'monospace', fontSize: 12, color: 'var(--text-primary)', cursor: 'pointer',
        height: ITEM_H,
    }

    return (
        <div
            ref={ref}
            role="menu"
            aria-label="Export note as"
            style={{
                position: 'fixed', left, top, width: MENU_W, zIndex: 1000,
                background: 'rgba(20,20,28,0.97)', borderRadius: 12,
                boxShadow: '0 8px 30px rgba(0,0,0,0.55), inset 0 0 0 1px rgba(167,139,250,0.2)',
                backdropFilter: 'blur(8px)', overflow: 'hidden',
            }}
        >
            {groups.map(({ group, items }, i) => (
                <div key={group} role="group" aria-label={GROUP_TITLES[group]} style={i > 0 ? { borderTop: '1px solid rgba(255,255,255,0.06)' } : undefined}>
                    <div style={{ padding: '8px 14px 2px', fontFamily: 'monospace', fontSize: 9, letterSpacing: '0.08em', textTransform: 'uppercase', color: 'var(--text-dim)', height: 26, boxSizing: 'border-box' }}>
                        {GROUP_TITLES[group]}
                    </div>
                    {items.map((o) => (
                        <button
                            key={o.format}
                            role="menuitem"
                            data-format={o.format}
                            style={{ ...itemStyle, opacity: busy && busy !== o.format ? 0.4 : 1 }}
                            disabled={busy !== null}
                            onClick={() => onPick(o.format)}
                        >
                            <span>{busy === o.format ? 'Exporting…' : o.label}</span>
                            <span style={{ color: 'var(--text-dim)', fontSize: 10 }}>.{o.extension}</span>
                        </button>
                    ))}
                </div>
            ))}
        </div>
    )
}
