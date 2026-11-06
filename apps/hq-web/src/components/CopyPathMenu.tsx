import { useEffect, useRef, useState } from 'react'
import { Check, Copy, Link2, Minus, Plus } from 'lucide-react'
import { t } from '~/strings'
import { getNote, getVaultRoot } from '~/lib/vaultApi'
import '~/components/vault/notes.css'
import { copyNoteLink, mintNoteLink } from '~/lib/noteLink'

/** The note's text size, shown in the menu on a note and changed from there. */
export interface ZoomControl {
    pct: number
    canSmaller: boolean
    canLarger: boolean
    modified: boolean
    onSmaller: () => void
    onLarger: () => void
    onReset: () => void
}

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
 * The More menu of a note: its link, its text, its path and the text size. Rendered when `menu` is set,
 * anchored under the button and clamped to the viewport.
 */
export function CopyPathMenu({ menu, onClose, onLinked, zoom }: {
    menu: CopyMenuState | null
    onClose: () => void
    /** Called once the note carries its id, so an open editor saves from the new version. */
    onLinked?: (path: string) => void
    zoom?: ZoomControl
}) {
    const [copied, setCopied] = useState<'link' | 'rel' | 'abs' | 'content' | null>(null)
    const [linkFailed, setLinkFailed] = useState(false)
    const ref = useRef<HTMLDivElement | null>(null)

    useEffect(() => {
        if (!menu) return
        setCopied(null)
        setLinkFailed(false)
        const dismiss = (e: Event) => {
            if (ref.current && e.target instanceof Node && ref.current.contains(e.target)) return
            onClose()
        }
        const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
        // Delay attach so the tap that opened the menu doesn't instantly close it.
        const timer = setTimeout(() => {
            window.addEventListener('pointerdown', dismiss)
            window.addEventListener('scroll', dismiss, true)
            window.addEventListener('keydown', onKey)
        }, 50)
        ref.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus()
        return () => {
            clearTimeout(timer)
            window.removeEventListener('pointerdown', dismiss)
            window.removeEventListener('scroll', dismiss, true)
            window.removeEventListener('keydown', onKey)
        }
    }, [menu, onClose])

    if (!menu) return null

    const MENU_W = 300
    const isNote = menu.relPath.endsWith('.md')
    const MENU_H = 56 * (isNote ? 5 : 3) + (zoom ? 56 : 0)
    const left = Math.max(8, Math.min(menu.x, window.innerWidth - MENU_W - 8))
    const top = Math.max(8, Math.min(menu.y, window.innerHeight - MENU_H - 8))

    const doCopyLink = async () => {
        try {
            const { copied: ok } = await copyNoteLink(menu.relPath, { mint: mintNoteLink, copy: copyText, origin: window.location.origin })
            onLinked?.(menu.relPath)
            if (!ok) return
            setCopied('link')
            setTimeout(onClose, 650)
        } catch {
            setLinkFailed(true)
        }
    }

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

    const row = (kind: 'rel' | 'abs' | 'content', label: string) => (
        <button type="button" role="menuitem" className="an-mi2" onClick={() => doCopy(kind)}>
            {copied === kind ? <Check className="a-ic" aria-hidden="true" /> : <Copy className="a-ic" aria-hidden="true" />}
            <span><b>{copied === kind ? t('notes_copied') : label}</b></span>
        </button>
    )

    return (
        <div ref={ref} role="menu" aria-label={t('notes_more_for_note')} className="a-menu" style={{ position: 'fixed', left, top, width: MENU_W, zIndex: 1000 }}>
            {isNote && (
                <button type="button" role="menuitem" className="an-mi2" onClick={doCopyLink}>
                    {copied === 'link' ? <Check className="a-ic" aria-hidden="true" /> : <Link2 className="a-ic" aria-hidden="true" />}
                    <span>
                        <b>{copied === 'link' ? t('notes_link_copied') : t('notes_copy_link')}</b>
                        <i style={linkFailed ? { color: 'var(--warn)' } : undefined}>
                            {linkFailed ? t('notes_link_copy_failed') : t('notes_copy_link_hint')}
                        </i>
                    </span>
                </button>
            )}
            {row('content', t('notes_copy_text'))}
            {row('rel', t('note_drawer_copy_path'))}
            {row('abs', t('notes_copy_full_path'))}
            {zoom && (
                <>
                    <div className="a-menu-sep" />
                    <div className="an-zoom" role="group" aria-label={t('notes_text_size')}>
                        <span>{t('notes_text_size')}</span>
                        <button type="button" className="a-ib" aria-label={t('notes_text_smaller')} disabled={!zoom.canSmaller} onClick={zoom.onSmaller}>
                            <Minus className="a-ic" aria-hidden="true" />
                        </button>
                        <output>{zoom.pct}%</output>
                        <button type="button" className="a-ib" aria-label={t('notes_text_larger')} disabled={!zoom.canLarger} onClick={zoom.onLarger}>
                            <Plus className="a-ic" aria-hidden="true" />
                        </button>
                        {zoom.modified && (
                            <button type="button" className="a-link" onClick={zoom.onReset}>{t('notes_text_reset')}</button>
                        )}
                    </div>
                </>
            )}
        </div>
    )
}
