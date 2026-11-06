import { useEffect, useRef, useSyncExternalStore } from 'react'
import { Sheet } from '~/components/ui/Sheet'
import '~/components/vault/notes.css'
import { t } from '~/strings'
import { exportKeys, type NoteExportFormat, type NoteExportGroup, type NoteExportOption } from '~/lib/noteExport'

export interface ExportMenuState {
    x: number
    y: number
}

const PHONE_QUERY = '(max-width: 767px)'
const MENU_W = 300
const ITEM_H = 56

const GROUP_KEYS: Record<NoteExportGroup, 'note_export_group_document' | 'note_export_group_picture' | 'note_export_group_tables'> = {
    document: 'note_export_group_document',
    image: 'note_export_group_picture',
    data: 'note_export_group_tables',
}

function subscribeNarrow(cb: () => void) {
    const mq = window.matchMedia(PHONE_QUERY)
    mq.addEventListener('change', cb)
    return () => mq.removeEventListener('change', cb)
}

/** True below 768 px, where the Export choices rise as a sheet instead of a menu. */
function useNarrow(): boolean {
    return useSyncExternalStore(subscribeNarrow, () => window.matchMedia(PHONE_QUERY).matches, () => false)
}

/**
 * The Export menu of a note, anchored under its chip and clamped to the viewport. It closes on an outside
 * tap, a scroll or Escape, and a file being made still arrives. `busy` is the format being made: its row
 * says so and the others wait.
 */
export function ExportMenu({
    menu,
    options,
    busy,
    title,
    onPick,
    onClose,
}: {
    menu: ExportMenuState | null
    options: readonly NoteExportOption[]
    busy: NoteExportFormat | null
    title: string
    onPick: (format: NoteExportFormat) => void
    onClose: () => void
}) {
    const ref = useRef<HTMLDivElement | null>(null)
    const narrow = useNarrow()

    useEffect(() => {
        if (!menu || narrow) return
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
    }, [menu, narrow, onClose])

    if (!menu) return null

    const groups = (['document', 'image', 'data'] as const)
        .map((group) => ({ group, items: options.filter((o) => o.group === group) }))
        .filter((g) => g.items.length > 0)
    const list = (
        <>
            {groups.map(({ group, items }, i) => (
                <div key={group} role="group" aria-label={t(GROUP_KEYS[group])}>
                    {i > 0 && <div className="a-menu-sep" />}
                    <p className="a-muted a-body-sm" style={{ padding: '4px 16px' }}>{t(GROUP_KEYS[group])}</p>
                    {items.map((o) => {
                        const keys = exportKeys(o.format)
                        return (
                            <button
                                key={o.format}
                                type="button"
                                role="menuitem"
                                data-format={o.format}
                                className="an-mi2"
                                style={{ opacity: busy && busy !== o.format ? 0.4 : 1 }}
                                disabled={busy !== null}
                                onClick={() => onPick(o.format)}
                            >
                                <span>
                                    {busy === o.format ? (
                                        <>
                                            <b>{t('note_export_making', { kind: t(keys.kind) })}</b>
                                            <i>{t('note_export_making_body')}</i>
                                        </>
                                    ) : (
                                        <>
                                            <b>{t(keys.name)}</b>
                                            <i>{t(keys.hint)}</i>
                                        </>
                                    )}
                                </span>
                                <code>.{o.extension}</code>
                            </button>
                        )
                    })}
                </div>
            ))}
        </>
    )
    if (narrow) return <Sheet title={t('notes_export_as')} onClose={onClose}>{list}</Sheet>

    const height = 40 + groups.length * 32 + options.length * ITEM_H
    const left = Math.max(8, Math.min(menu.x, window.innerWidth - MENU_W - 8))
    const top = Math.max(8, Math.min(menu.y, window.innerHeight - height - 8))

    return (
        <div
            ref={ref}
            role="menu"
            aria-label={t('note_export_title', { title })}
            className="a-menu"
            style={{ position: 'fixed', left, top, width: MENU_W, zIndex: 1000, maxHeight: 'calc(100vh - 16px)', overflowY: 'auto' }}
        >
            <p className="a-label" style={{ padding: '4px 16px' }}>{t('note_export_title', { title })}</p>
            {list}
        </div>
    )
}
