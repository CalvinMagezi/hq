import { Link } from '@tanstack/react-router'
import { Check, ChevronDown, ChevronRight, RefreshCw, Search } from 'lucide-react'
import { Fragment, useState } from 'react'
import { Panes, PaneHead } from '~/components/ui/Panes'
import { groupByDay } from '~/lib/inboxCard'
import type { InboxTab, NotificationItem, SilenceTopic } from '~/lib/notificationsApi'
import { t, tn, type StringKey } from '~/strings'
import { digestOf, splitHelpers } from '~/lib/inboxView'
import type { Pile } from '~/lib/catchUp'
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
  /** Set when an update is unread: marks them all read and leaves them listed. */
  onReadAll?: () => void
  /** Set on Done when it lists something: empties the list. */
  onClearDone?: () => void
  /** Updates the owner said Later to; they wait out of the list until their time. */
  later?: NotificationItem[]
  onLater?: (id: string) => void
  onBringBack?: (id: string) => void
  /** Sites the owner allowed once and for all; Done lists them so removing one is a visible button. */
  sites?: string[]
  onForgetSite?: (site: string) => void
  /** More than five requests wait and some are from helpers that are done: one card offers to clear those. */
  pile?: Pile | null
  pileBusy?: boolean
  onClearOld?: () => void
  /** Whether the list shows only what needs the person, with the old ones set aside. */
  onlyNeeds?: boolean
  onToggleNeeds?: () => void
  /** Agent-dialog notices cleared from the Inbox, listed in Done with Restore. */
  cleared?: NotificationItem[]
  onRestore?: (ids: string[]) => void
  /** Silences the topic of the open item. */
  onSilence?: (topic: SilenceTopic) => void
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

/** A closed row that opens to a short list: the helpers' requests, the helper digest, what waits for later. */
function Fold({ title, line, children }: { title: string; line: string; children: React.ReactNode }) {
  const [open, setOpen] = useState(false)
  const Chevron = open ? ChevronDown : ChevronRight
  return (
    <div className="hq-fold">
      <button type="button" className="a-li hq-fold-head" aria-expanded={open} onClick={() => setOpen(!open)}>
        <span className="a-li-main">
          <b>{title}</b>
          <span>{line}</span>
        </span>
        <Chevron className="a-ic sm" aria-hidden="true" />
      </button>
      {open && <div className="hq-fold-body">{children}</div>}
    </div>
  )
}

function UpdateDays({ items, now, selectedId, onClear, busy }: { items: NotificationItem[]; now: number; selectedId?: string; onClear?: (id: string) => void; busy: boolean }) {
  return (
    <>
      {groupByDay(items, now).map((group) => (
        <Fragment key={group.heading}>
          <p className="a-grp">{group.heading}</p>
          {(() => {
            const { rows, helpers } = digestOf(group.items)
            return (
              <>
                {helpers.length > 0 && (
                  <Fold title={tn('inbox_digest', helpers.length)} line={t('inbox_digest_line')}>
                    {helpers.map((n) => (
                      <UpdateRow key={n.id} notification={n} selected={n.id === selectedId} busy={busy} onClear={onClear && n.state === 'pending' ? () => onClear(n.id) : undefined} />
                    ))}
                  </Fold>
                )}
                {rows.map((n) => (
                  <UpdateRow key={n.id} notification={n} selected={n.id === selectedId} busy={busy} onClear={onClear && n.state === 'pending' ? () => onClear(n.id) : undefined} />
                ))}
              </>
            )
          })()}
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
  // Requests from helpers fold into one quiet row under the owner's own.
  const { mine, helpers: fromHelpers } = p.tab === 'pending' ? splitHelpers(p.items) : { mine: p.items, helpers: [] }

  const list = (
    <>
      <PaneHead title={t('inbox_title')}>
        <button type="button" className="a-ib" disabled={p.refreshing} aria-label={t('inbox_refresh_feed')} onClick={p.onRefresh}>
          <RefreshCw className={`a-ic${p.refreshing ? ' a-spin' : ''}`} aria-hidden="true" />
        </button>
      </PaneHead>
      <nav className="hq-inbox-doors" aria-label={t('inbox_doors_label')}>
        <Link to="/tasks" className="a-btn quiet compact">
          {t('inbox_door_tasks')}
        </Link>
        <Link to="/helpers" className="a-btn quiet compact">
          {t('inbox_door_helpers')}
        </Link>
      </nav>
      <div className="a-tabs" role="tablist" aria-label={t('inbox_title')}>
        {TABS.map(({ id, label }) => (
          <button key={id} type="button" role="tab" className="a-tab" aria-selected={p.tab === id} onClick={() => p.onTab(id)}>
            {t(label)}
            {id === 'pending' && tabCount(id) > 0 && <em className="need">{tabCount(id)}</em>}
            {/* Only Needs you carries a number; Updates has a quiet dot when something is unread, Done nothing. */}
            {id === 'all' && tabCount(id) > 0 && <span className="a-dot hq-tab-dot" role="img" aria-label={t('inbox_updates_unread')} />}
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
            {(p.onClearAll || p.onReadAll) && (
              <div className="hq-inbox-clear">
                {p.onReadAll && (
                  <button type="button" className="a-btn quiet compact" onClick={p.onReadAll}>
                    {t('inbox_read_all')}
                  </button>
                )}
                {p.onClearAll && (
                  <button type="button" className="a-btn quiet compact" onClick={p.onClearAll}>
                    {t('inbox_clear_all_updates')}
                  </button>
                )}
              </div>
            )}
          </>
        )}
        {p.tab === 'resolved' && p.onClearDone && (
          <div className="hq-inbox-clear">
            <button type="button" className="a-btn quiet compact" onClick={p.onClearDone}>
              {t('inbox_clear_done')}
            </button>
          </div>
        )}
        {p.tab === 'pending' && p.pile && (
          <section className="a-card hq-pile" aria-label={t('inbox_pile_title', { n: String(p.pile.waiting) })}>
            <b>{t('inbox_pile_title', { n: String(p.pile.waiting) })}</b>
            <p className="a-body-sm">{t('inbox_pile_line', { old: String(p.pile.old) })}</p>
            <div className="a-btns">
              <button type="button" className="a-btn secondary" disabled={p.pileBusy} onClick={p.onClearOld}>
                {t('inbox_pile_clear')}
              </button>
              <button type="button" className="a-btn quiet" onClick={p.onToggleNeeds}>
                {p.onlyNeeds ? t('inbox_pile_show_all') : t('inbox_pile_show', { n: String(p.pile.rest) })}
              </button>
            </div>
          </section>
        )}
        {p.tab === 'pending' ? (
          <div className="hq-inbox-list">
            <NotifyCard />
            {mine.map((n) => (
              <NotificationCard key={n.id} notification={n} selected={n.id === p.selected?.id} onAction={p.onAction} onAnswered={p.onAnswered} loadingAction={p.loadingAction} />
            ))}
            {fromHelpers.length > 0 && (
              <Fold title={t('inbox_helpers_fold', { n: String(fromHelpers.length) })} line={t('inbox_helpers_fold_line')}>
                {fromHelpers.map((n) => (
                  <UpdateRow key={n.id} notification={n} selected={n.id === p.selected?.id} busy={p.loadingAction !== null} />
                ))}
              </Fold>
            )}
          </div>
        ) : (
          <>
            {p.tab === 'all' && p.later && p.later.length > 0 && (
              <Fold title={t('inbox_waiting_later', { n: String(p.later.length) })} line={t('inbox_waiting_line')}>
                {p.later.map((n) => (
                  <div key={n.id} className="hq-later-row">
                    <UpdateRow notification={n} selected={n.id === p.selected?.id} />
                    <button type="button" className="a-btn quiet compact" onClick={() => p.onBringBack?.(n.id)}>
                      {t('inbox_bring_back')}
                    </button>
                  </div>
                ))}
              </Fold>
            )}
            {p.tab === 'resolved' && p.cleared && p.cleared.length > 0 && (
              <Fold title={t('inbox_gone_title', { n: String(p.cleared.length) })} line={t('inbox_gone_line')}>
                {p.cleared.map((n) => (
                  <div key={n.id} className="hq-later-row">
                    <UpdateRow notification={n} />
                    <button type="button" className="a-btn quiet compact" onClick={() => p.onRestore?.([n.id])}>
                      {t('inbox_gone_restore')}
                    </button>
                  </div>
                ))}
                <button type="button" className="a-btn secondary compact" onClick={() => p.onRestore?.(p.cleared?.map((n) => n.id) ?? [])}>
                  {t('inbox_gone_restore_all', { n: String(p.cleared.length) })}
                </button>
              </Fold>
            )}
            {p.tab === 'resolved' && p.sites && p.sites.length > 0 && (
              <section aria-label={t('inbox_sites_title')}>
                <p className="a-grp">{t('inbox_sites_title')}</p>
                {p.sites.map((site) => (
                  <div key={site} className="a-li">
                    <span className="a-li-main">
                      <b>{t('inbox_site_line', { site })}</b>
                      <span>{t('inbox_site_allowed')}</span>
                    </span>
                    <button type="button" className="a-btn secondary compact" onClick={() => p.onForgetSite?.(site)}>
                      {t('inbox_site_remove')}
                    </button>
                  </div>
                ))}
              </section>
            )}
          <UpdateDays items={shown} now={now} selectedId={p.selected?.id} onClear={p.tab === 'all' ? p.onClear : undefined} busy={p.loadingAction !== null} />
          </>
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
      detail={<NotificationDetail notification={p.selected} onAction={p.onAction} onAnswered={p.onAnswered} loadingAction={p.loadingAction} onOpenLink={p.onOpenLink} onSilence={p.onSilence} onLater={p.onLater} />}
    />
  )
}
