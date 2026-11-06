import { Check, RefreshCw, Search } from 'lucide-react'
import { Fragment } from 'react'
import { Panes, PaneHead } from '~/components/ui/Panes'
import { groupByDay } from '~/lib/inboxCard'
import type { InboxTab, NotificationItem } from '~/lib/notificationsApi'
import { t, type StringKey } from '~/strings'
import { NotificationCard } from './NotificationCard'
import { NotificationDetail } from './NotificationDetail'
import { NotifyCard } from './NotifyCard'
import { UpdateRow, clockTime } from './UpdateRow'
import { UPDATE_FILTERS, countByKind, updateKind, type UpdateFilter } from './updateKind'

const CHIP_LABEL: Record<UpdateFilter, StringKey> = {
  all: 'search_chip_all',
  tasks: 'search_chip_tasks',
  helpers: 'search_chip_helpers',
  hq: 'inbox_chip_hq',
}

const TABS: { id: InboxTab; label: StringKey }[] = [
  { id: 'pending', label: 'inbox_tab_needs' },
  { id: 'all', label: 'inbox_tab_updates' },
  { id: 'resolved', label: 'inbox_tab_done' },
]

export interface InboxViewProps {
  tab: InboxTab
  onTab: (tab: InboxTab) => void
  /** Items of the open tab, already narrowed by the search. */
  items: NotificationItem[]
  needsCount: number
  updatesCount: number
  query: string
  onQuery: (query: string) => void
  filter: UpdateFilter
  onFilter: (filter: UpdateFilter) => void
  selected: NotificationItem | null
  /** When the list last came from HQ; the caught-up line waits for it. */
  checkedAt: Date | null
  refreshing: boolean
  onRefresh: () => void
  /** Set when there are updates to clear in one go. */
  onClearAll?: () => void
  onAction(id: string, action: string): Promise<void>
  /** Clears one update; it reports its own failure. */
  onClear: (id: string) => void
  onAnswered: (id: string, choice: 'approve' | 'decline') => void
  loadingAction: string | null
  onOpenLink: (link: string) => void
  now?: number
}

function Empty({ children }: { children: string }) {
  return (
    <div className="a-empty">
      <p>{children}</p>
    </div>
  )
}

function UpdateDays({ items, now, selectedId, onClear, busy }: { items: NotificationItem[]; now: number; selectedId?: string; onClear?: (id: string) => void; busy: boolean }) {
  return (
    <>
      {groupByDay(items, now).map((group) => (
        <Fragment key={group.heading}>
          <p className="a-grp">{group.heading}</p>
          {group.items.map((n) => (
            <UpdateRow key={n.id} notification={n} selected={n.id === selectedId} busy={busy} onClear={onClear && n.state === 'pending' ? () => onClear(n.id) : undefined} />
          ))}
        </Fragment>
      ))}
    </>
  )
}

/** The Inbox on the design's panes: tabs, search and the list on the left, the open item on the right. */
export function InboxView(p: InboxViewProps) {
  const now = p.now ?? Date.now()
  const counts = countByKind(p.tab === 'all' ? p.items : [])
  const shown = p.tab === 'all' && p.filter !== 'all' ? p.items.filter((n) => updateKind(n) === p.filter) : p.items
  const searching = p.query.trim() !== ''
  const tabCount = (id: InboxTab) => (id === 'pending' ? p.needsCount : id === 'all' ? p.updatesCount : 0)

  const list = (
    <>
      <PaneHead title={t('inbox_title')}>
        <button type="button" className="a-ib" disabled={p.refreshing} aria-label={t('inbox_refresh_feed')} onClick={p.onRefresh}>
          <RefreshCw className={`a-ic${p.refreshing ? ' a-spin' : ''}`} aria-hidden="true" />
        </button>
      </PaneHead>
      <div className="a-tabs" role="tablist" aria-label={t('inbox_title')}>
        {TABS.map(({ id, label }) => (
          <button key={id} type="button" role="tab" className="a-tab" aria-selected={p.tab === id} onClick={() => p.onTab(id)}>
            {t(label)}
            {tabCount(id) > 0 && <em className={id === 'pending' ? 'need' : undefined}>{tabCount(id)}</em>}
          </button>
        ))}
      </div>
      <div className="hq-inbox-tools">
        <label className="a-search">
          <Search className="a-ic sm" aria-hidden="true" />
          <span className="a-sr">{t('inbox_search')}</span>
          <input type="search" value={p.query} onChange={(e) => p.onQuery(e.target.value)} placeholder={t('inbox_search')} />
        </label>
      </div>
      <div className="hq-inbox-scroll">
        {p.tab === 'all' && (
          <>
            <div className="a-chips" role="group" aria-label={t('tasks_show')}>
              {UPDATE_FILTERS.map((f) => (
                <button key={f} type="button" className="a-chip" aria-pressed={p.filter === f} onClick={() => p.onFilter(f)}>
                  {t(CHIP_LABEL[f])} <em>{counts[f]}</em>
                </button>
              ))}
            </div>
            {p.onClearAll && (
              <div className="hq-inbox-clear">
                <button type="button" className="a-btn quiet compact" onClick={p.onClearAll}>
                  {t('inbox_clear_all_updates')}
                </button>
              </div>
            )}
          </>
        )}
        {p.tab === 'pending' ? (
          <div className="hq-inbox-list">
            <NotifyCard />
            {p.items.map((n) => (
              <NotificationCard key={n.id} notification={n} selected={n.id === p.selected?.id} onAction={p.onAction} onAnswered={p.onAnswered} loadingAction={p.loadingAction} />
            ))}
          </div>
        ) : (
          <UpdateDays items={shown} now={now} selectedId={p.selected?.id} onClear={p.tab === 'all' ? p.onClear : undefined} busy={p.loadingAction !== null} />
        )}
        {shown.length === 0 && searching && <Empty>{t('inbox_none_found')}</Empty>}
        {shown.length === 0 && !searching && p.tab === 'pending' && p.checkedAt && (
          <div className="a-empty">
            <span className="a-empty-ic">
              <Check className="a-ic" aria-hidden="true" />
            </span>
            <b>{t('inbox_you_re_all_caught')}</b>
            <p>{t('inbox_caught_up_body', { time: clockTime(p.checkedAt) })}</p>
          </div>
        )}
        {shown.length === 0 && !searching && p.tab === 'all' && <Empty>{t('inbox_no_updates')}</Empty>}
        {shown.length === 0 && !searching && p.tab === 'resolved' && <Empty>{t('inbox_nothing_decided_yet')}</Empty>}
      </div>
    </>
  )

  return (
    <Panes
      list={list}
      showDetail={!!p.selected}
      listLabel={t('inbox_title')}
      detailLabel={t('inbox_details')}
      detail={<NotificationDetail notification={p.selected} onAction={p.onAction} onAnswered={p.onAnswered} loadingAction={p.loadingAction} onOpenLink={p.onOpenLink} />}
    />
  )
}
