import { createFileRoute, Link } from '@tanstack/react-router'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { vaultHealthQuery } from '~/lib/queries'
import { useHealthChange } from '~/lib/useHealthChange'
import { useRefreshOn } from '~/lib/useRefreshOn'
import {
    applyHealthFixes,
    countHealthAgain,
    ignoreHealthItem,
    problemCount,
    scanVaultHealth,
    setAsideHealth,
    undoHealthRepair,
    unfixableCount,
    type HealthRepair,
    type VaultHealth,
} from '~/lib/vaultHealthApi'
import { healthSections, skipText, type HealthItem, type HealthSection } from '~/lib/vaultHealthItems'
import { relTime } from '~/lib/time'
import { toastAlert } from '~/lib/toast'
import { Panes, PaneHead } from '~/components/ui/Panes'
import { BackLink } from '~/components/vault/NoteViews'
import { folderTrail, noteName } from '~/components/vault/noteRows'
import { t, tn } from '~/strings'
import '~/components/vault/notes.css'

const NOTEBOOKS = 'Notebooks'

export const Route = createFileRoute('/notes/health')({
    component: VaultHealthPage,
    ssr: false,
})

interface RowProps {
    item: HealthItem
    busy: boolean
    onIgnore: (item: HealthItem) => void
    onFix?: (item: HealthItem) => void
}

/** What a fix would do, in a line: the link as it would read, or what happens to a repeated note. */
function fixLine(item: HealthItem): string | null {
    const f = item.fix
    if (!f) return null
    if (f.kind === 'merge_duplicate') return t('notes_health_merge', { name: noteName(f.after) })
    if (f.kind === 'nested_frontmatter') return t('notes_health_merge_props')
    return t('notes_health_becomes', { after: f.after })
}

/** One thing to look at: the link or names, where it is, and a way to open the note, fix it or leave it be. */
export function HealthRow({ item, busy, onIgnore, onFix }: RowProps) {
    const place = folderTrail(item.note) || t('notes_your_notes')
    const fix = fixLine(item)
    return (
        <div className="a-li" data-health-item>
            <div className="a-li-main">
                <b className="l-clip">{item.text}</b>
                <span className="a-body-sm a-muted">
                    {place}
                    {item.line !== null && <> · {t('notes_health_line', { n: item.line })}</>}
                </span>
                {item.detail && <span className="a-body-sm a-muted">{item.detail}</span>}
                {fix && (
                    <span className="a-body-sm" data-health-fix>
                        <em>{item.fix?.tier === 'a' ? t('notes_health_tier_a') : t('notes_health_tier_b')}</em> · {fix}
                    </span>
                )}
            </div>
            {item.fix && onFix && (
                <button type="button" className="a-btn compact" disabled={busy} onClick={() => onFix(item)}>
                    {t('notes_health_fix')}
                </button>
            )}
            <Link to="/notes/$" params={{ _splat: item.note }} className="a-btn compact secondary">
                {t('notes_health_open')}
            </Link>
            <button type="button" className="a-btn compact quiet" disabled={busy} onClick={() => onIgnore(item)}>
                {t('notes_health_ignore')}
            </button>
        </div>
    )
}

interface ViewProps {
    health: VaultHealth | undefined
    loadFailed: boolean
    scanning: boolean
    busyKey: string | null
    now?: number
    onScan: () => void
    onIgnore: (item: HealthItem) => void
    onFix?: (item: HealthItem) => void
    onFixAll?: () => void
    onUndo?: (run: HealthRepair) => void
    onSetAside?: () => void
    onCountAgain?: () => void
}

/** The whole page, from data: when it was checked, what was found, and each thing to look at. */
export function VaultHealthView({ health, loadFailed, scanning, busyKey, now, onScan, onIgnore, onFix, onFixAll, onUndo, onSetAside, onCountAgain }: ViewProps) {
    const report = health?.report ?? null
    const sections: HealthSection[] = report ? healthSections(report, health?.fixes ?? []) : []
    const certain = health?.fix_counts?.a ?? 0
    const showFixAll = certain > 0 && onFixAll !== undefined
    const skipped = health?.skipped ?? []
    const setAside = report?.counts.set_aside ?? 0
    const offerLeaveAlone = onSetAside !== undefined && unfixableCount(health) > 0
    const hasAside = setAside > 0
    const showAsideRow = hasAside || offerLeaveAlone
    const repairs = (health?.repairs ?? []).slice(0, 5)
    return (
        <div className="an-home">
            <p className="a-body-sm a-muted">{t('notes_health_intro')}</p>
            <div className="l-row justify-between">
                <span className="a-body-sm a-muted">
                    {health?.checked_at ? t('notes_health_checked', { when: relTime(health.checked_at, now) }) : t('notes_health_not_checked')}
                </span>
                <button type="button" className="a-btn compact secondary" disabled={scanning} onClick={onScan}>
                    {scanning ? t('notes_health_checking') : t('notes_health_check_now')}
                </button>
            </div>
            {showFixAll && (
                <div className="l-row justify-between">
                    <span className="a-body-sm">{tn('notes_health_fix_count', certain)}</span>
                    <button type="button" className="a-btn compact" disabled={busyKey === 'all'} onClick={onFixAll}>
                        {busyKey === 'all' ? t('notes_health_fixing') : t('notes_health_fix')}
                    </button>
                </div>
            )}
            {showAsideRow && (
                <div className="l-row justify-between" data-health-aside>
                    <span className="a-body-sm a-muted">
                        {hasAside ? tn('notes_health_set_aside', setAside) : t('notes_health_leave_alone_hint')}
                    </span>
                    <span className="l-row">
                        {hasAside && onCountAgain && (
                            <button type="button" className="a-btn compact secondary" disabled={busyKey === 'all'} onClick={onCountAgain}>
                                {t('notes_health_count_again')}
                            </button>
                        )}
                        {offerLeaveAlone && (
                            <button type="button" className="a-btn compact secondary" disabled={busyKey === 'all'} onClick={onSetAside}>
                                {t('notes_health_leave_alone')}
                            </button>
                        )}
                    </span>
                </div>
            )}
            {loadFailed && <p role="alert" className="a-body-sm a-muted">{t('notes_health_load_failed')}</p>}
            {report && problemCount(report.counts) === 0 && (
                <p className="a-body-sm">{t('notes_health_all_good')}</p>
            )}
            {sections.map((s) => {
                const hidden = s.total - s.items.length
                return (
                <section key={s.id} aria-label={tn(s.titleKey, s.total)} data-health-section={s.id}>
                    <p className="a-grp">{tn(s.titleKey, s.total)}</p>
                    <div className="a-set" style={{ margin: 0 }}>
                        {s.items.map((item) => (
                            <HealthRow key={item.id} item={item} busy={busyKey === `${item.kind}:${item.key}` || busyKey === 'all'} onIgnore={onIgnore} onFix={onFix} />
                        ))}
                    </div>
                    {hidden > 0 && <p className="a-body-sm a-muted">{t('notes_health_more', { n: hidden })}</p>}
                </section>
                )
            })}
            {skipped.length > 0 && (
                <section aria-label={tn('notes_health_skipped', skipped.length)} data-health-section="skipped">
                    <p className="a-grp">{tn('notes_health_skipped', skipped.length)}</p>
                    <div className="a-set" style={{ margin: 0 }}>
                        {skipped.slice(0, 20).map((k) => (
                            <div key={k.note} className="a-li" data-health-skipped>
                                <div className="a-li-main">
                                    <b className="l-clip">{noteName(k.note)}</b>
                                    <span className="a-body-sm a-muted">{skipText(k.why)}</span>
                                </div>
                            </div>
                        ))}
                    </div>
                </section>
            )}
            {repairs.length > 0 && (
                <section aria-label={t('notes_health_history')} data-health-section="history">
                    <p className="a-grp">{t('notes_health_history')}</p>
                    <div className="a-set" style={{ margin: 0 }}>
                        {repairs.map((r) => (
                            <div key={r.run_id} className="a-li" data-health-run>
                                <div className="a-li-main">
                                    <span className="a-body-sm">{t('notes_health_run_line', { when: relTime(r.ran_at, now), n: r.fixes })}</span>
                                    {r.source === 'auto' && <span className="a-body-sm a-muted">{t('notes_health_run_by_hq')}</span>}
                                </div>
                                {r.undone_at ? (
                                    <span className="a-body-sm a-muted">{t('notes_health_run_taken_back')}</span>
                                ) : (
                                    onUndo && (
                                        <button type="button" className="a-btn compact secondary" disabled={busyKey === 'all'} onClick={() => onUndo(r)}>
                                            {t('notes_health_undo')}
                                        </button>
                                    )
                                )}
                            </div>
                        ))}
                    </div>
                </section>
            )}
        </div>
    )
}

function VaultHealthPage() {
    const queryClient = useQueryClient()
    const q = useQuery(vaultHealthQuery)
    useRefreshOn(['vault-health:change'], () => void q.refetch())
    const [scanning, setScanning] = useState(false)
    const { busyKey, change } = useHealthChange()

    const scan = async () => {
        setScanning(true)
        try {
            queryClient.setQueryData(vaultHealthQuery.queryKey, await scanVaultHealth())
        } catch {
            toastAlert(t('notes_health_scan_failed'))
        } finally {
            setScanning(false)
        }
    }
    const ignore = (item: HealthItem) =>
        change(
            `${item.kind}:${item.key}`,
            async () => ({ health: await ignoreHealthItem(item.kind, item.key), message: '' }),
            t('notes_health_ignore_failed'),
        )
    const fixOne = (item: HealthItem) =>
        item.fix &&
        change(
            `${item.kind}:${item.key}`,
            async () => {
                const r = await applyHealthFixes({ ids: [item.fix!.id] })
                return { health: r.health, message: tn('notes_health_fixed', r.applied.fixes.length) }
            },
            t('notes_health_fix_failed'),
        )
    const fixAll = () =>
        change(
            'all',
            async () => {
                const r = await applyHealthFixes({ tier: 'a' })
                return { health: r.health, message: tn('notes_health_fixed', r.applied.fixes.length) }
            },
            t('notes_health_fix_failed'),
        )
    const undo = (run: HealthRepair) =>
        change(
            'all',
            async () => {
                const r = await undoHealthRepair(run.run_id)
                const kept = r.undone.edited_since.length
                return {
                    health: r.health,
                    message: kept > 0 ? tn('notes_health_undo_kept', kept) : tn('notes_health_undone', r.undone.restored.length + r.undone.untrashed.length),
                }
            },
            t('notes_health_undo_failed'),
        )

    const setAside = () =>
        change(
            'all',
            async () => {
                const h = await setAsideHealth()
                return { health: h, message: t('notes_health_set_aside_done', { n: h.set_aside_now ?? 0 }) }
            },
            t('notes_health_leave_alone_failed'),
        )
    const countAgain = () =>
        change(
            'all',
            async () => ({ health: await countHealthAgain(), message: t('notes_health_counted_again') }),
            t('notes_health_count_again_failed'),
        )

    return (
        <Panes
            detailLabel={t('notes_health_page')}
            detail={
                <>
                    <PaneHead title={t('notes_health_page')} back={<BackLink path={NOTEBOOKS} />} />
                    <VaultHealthView
                        health={q.data}
                        loadFailed={q.isError}
                        scanning={scanning}
                        busyKey={busyKey}
                        onScan={() => void scan()}
                        onIgnore={(item) => void ignore(item)}
                        onFix={(item) => void fixOne(item)}
                        onFixAll={() => void fixAll()}
                        onUndo={(run) => void undo(run)}
                        onSetAside={() => void setAside()}
                        onCountAgain={() => void countAgain()}
                    />
                </>
            }
        />
    )
}
