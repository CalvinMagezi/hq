import { createFileRoute, Link } from '@tanstack/react-router'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { vaultHealthQuery } from '~/lib/queries'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { ignoreHealthItem, problemCount, scanVaultHealth, type VaultHealth } from '~/lib/vaultHealthApi'
import { healthSections, type HealthItem, type HealthSection } from '~/lib/vaultHealthItems'
import { relTime } from '~/lib/time'
import { toastAlert } from '~/lib/toast'
import { Panes, PaneHead } from '~/components/ui/Panes'
import { BackLink } from '~/components/vault/NoteViews'
import { folderTrail } from '~/components/vault/noteRows'
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
}

/** One thing to look at: the link or names, where it is, and a way to open the note or leave it be. */
export function HealthRow({ item, busy, onIgnore }: RowProps) {
    const place = folderTrail(item.note) || t('notes_your_notes')
    return (
        <div className="a-li" data-health-item>
            <div className="a-li-main">
                <b className="l-clip">{item.text}</b>
                <span className="a-body-sm a-muted">
                    {place}
                    {item.line !== null && <> · {t('notes_health_line', { n: item.line })}</>}
                </span>
                {item.detail && <span className="a-body-sm a-muted">{item.detail}</span>}
            </div>
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
}

/** The whole page, from data: when it was checked, what was found, and each thing to look at. */
export function VaultHealthView({ health, loadFailed, scanning, busyKey, now, onScan, onIgnore }: ViewProps) {
    const report = health?.report ?? null
    const sections: HealthSection[] = report ? healthSections(report) : []
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
            {loadFailed && <p role="alert" className="a-body-sm a-muted">{t('notes_health_load_failed')}</p>}
            {report && problemCount(report.counts) === 0 && sections.length === 0 && (
                <p className="a-body-sm">{t('notes_health_all_good')}</p>
            )}
            {sections.map((s) => {
                const hidden = s.total - s.items.length
                return (
                <section key={s.id} aria-label={tn(s.titleKey, s.total)} data-health-section={s.id}>
                    <p className="a-grp">{tn(s.titleKey, s.total)}</p>
                    <div className="a-set" style={{ margin: 0 }}>
                        {s.items.map((item) => (
                            <HealthRow key={item.id} item={item} busy={busyKey === `${item.kind}:${item.key}`} onIgnore={onIgnore} />
                        ))}
                    </div>
                    {hidden > 0 && <p className="a-body-sm a-muted">{t('notes_health_more', { n: hidden })}</p>}
                </section>
                )
            })}
        </div>
    )
}

function VaultHealthPage() {
    const queryClient = useQueryClient()
    const q = useQuery(vaultHealthQuery)
    useRefreshOn(['vault-health:change'], () => void q.refetch())
    const [scanning, setScanning] = useState(false)
    const [busyKey, setBusyKey] = useState<string | null>(null)

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
    const ignore = async (item: HealthItem) => {
        setBusyKey(`${item.kind}:${item.key}`)
        try {
            queryClient.setQueryData(vaultHealthQuery.queryKey, await ignoreHealthItem(item.kind, item.key))
        } catch {
            toastAlert(t('notes_health_ignore_failed'))
        } finally {
            setBusyKey(null)
        }
    }

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
                    />
                </>
            }
        />
    )
}
