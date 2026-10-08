import React, { useEffect, useRef, useState } from 'react'
import { getNote, getVaultRoot } from '~/lib/vaultApi'

export interface CopyMenuState {
    relPath: string
    x: number
    y: number
}

async function copyText(text: string): Promise<boolean> {
    try {
        await navigator.clipboard.writeText(text)
        return true
    } catch {
        // Fallback for older WebViews / non-secure contexts
        const ta = document.createElement('textarea')
        ta.value = text
        ta.style.position = 'fixed'
        ta.style.opacity = '0'
        document.body.appendChild(ta)
        ta.select()
        let ok = false
        try { ok = document.execCommand('copy') } catch { ok = false }
        document.body.removeChild(ta)
        return ok
    }
}

/**
 * Floating "copy" menu (relative path, absolute path, note content). Rendered when `menu` is set; anchors near the
 * long-press point, clamped to the viewport so it stays usable on phones.
 */
export function CopyPathMenu({ menu, onClose }: { menu: CopyMenuState | null; onClose: () => void }) {
    const [copied, setCopied] = useState<'rel' | 'abs' | 'content' | null>(null)
    const ref = useRef<HTMLDivElement | null>(null)

    useEffect(() => {
        if (!menu) return
        setCopied(null)
        const dismiss = (e: Event) => {
            if (ref.current && e.target instanceof Node && ref.current.contains(e.target)) return
            onClose()
        }
        const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
        // Delay attach so the long-press touchend/click doesn't instantly close it
        const t = setTimeout(() => {
            window.addEventListener('pointerdown', dismiss)
            window.addEventListener('scroll', dismiss, true)
            window.addEventListener('keydown', onKey)
        }, 50)
        return () => {
            clearTimeout(t)
            window.removeEventListener('pointerdown', dismiss)
            window.removeEventListener('scroll', dismiss, true)
            window.removeEventListener('keydown', onKey)
        }
    }, [menu, onClose])

    if (!menu) return null

    const MENU_W = 220
    const MENU_H = 144
    const left = Math.max(8, Math.min(menu.x, window.innerWidth - MENU_W - 8))
    const top = Math.max(8, Math.min(menu.y, window.innerHeight - MENU_H - 8))

    const doCopy = async (kind: 'rel' | 'abs' | 'content') => {
        let text = menu.relPath
        if (kind === 'content') {
            try {
                const note = await getNote(menu.relPath)
                if (note.isDir) return
                text = note.content
            } catch {
                return
            }
        } else if (kind === 'abs') {
            try {
                const res = await getVaultRoot()
                text = `${res.path.replace(/\/$/, '')}/${menu.relPath}`
            } catch {
                text = menu.relPath
            }
        }
        if (await copyText(text)) {
            setCopied(kind)
            setTimeout(onClose, 650)
        }
    }

    const itemStyle: React.CSSProperties = {
        display: 'flex', alignItems: 'center', gap: 8, width: '100%',
        padding: '10px 14px', background: 'none', border: 'none', textAlign: 'left',
        fontFamily: 'monospace', fontSize: 12, color: 'var(--text-primary)', cursor: 'pointer',
        minHeight: 40,
    }

    return (
        <div
            ref={ref}
            role="menu"
            style={{
                position: 'fixed', left, top, width: MENU_W, zIndex: 1000,
                background: 'rgba(20,20,28,0.97)', borderRadius: 12,
                boxShadow: '0 8px 30px rgba(0,0,0,0.55), inset 0 0 0 1px rgba(167,139,250,0.2)',
                backdropFilter: 'blur(8px)', overflow: 'hidden',
            }}
        >
            <button role="menuitem" style={itemStyle} onClick={() => doCopy('rel')}>
                <span style={{ color: 'var(--accent-blue)' }}>{copied === 'rel' ? '✓' : '⧉'}</span>
                {copied === 'rel' ? 'Copied!' : 'Copy relative path'}
            </button>
            <button role="menuitem" style={{ ...itemStyle, borderTop: '1px solid rgba(255,255,255,0.06)' }} onClick={() => doCopy('abs')}>
                <span style={{ color: 'var(--accent-blue)' }}>{copied === 'abs' ? '✓' : '⧉'}</span>
                {copied === 'abs' ? 'Copied!' : 'Copy absolute path'}
            </button>
            <button role="menuitem" style={{ ...itemStyle, borderTop: '1px solid rgba(255,255,255,0.06)' }} onClick={() => doCopy('content')}>
                <span style={{ color: 'var(--accent-blue)' }}>{copied === 'content' ? '✓' : '⧉'}</span>
                {copied === 'content' ? 'Copied!' : 'Copy note content'}
            </button>
        </div>
    )
}
