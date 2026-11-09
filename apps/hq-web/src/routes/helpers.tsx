import { t } from '~/strings'
import { createFileRoute, useParams } from '@tanstack/react-router'
import { MoreHorizontal, PanelLeftClose, Plus, Terminal, TriangleAlert } from 'lucide-react'
import { useNavigate } from '@tanstack/react-router'
import { useEffect, useMemo, useState } from 'react'
import { NewAgentDialog } from '~/components/sessions/NewAgentDialog'
import { LockdownBanner, StopAllDialog } from '~/components/sessions/EmergencyStop'
import { SessionDetail } from '~/components/sessions/SessionDetail'
import { SessionInspector } from '~/components/sessions/SessionInspector'
import { SessionRow } from '~/components/sessions/SessionRow'
import { usePolled } from '~/components/sessions/usePolled'
import { Panes } from '~/components/ui/Panes'
import { globalSessionsApi, workbenchApi, type HarnessSession, type WorkbenchHost } from '~/lib/sessionsApi'
import { useHQStore } from '~/store/hqStore'
import { needsYouCount, workingCount } from '~/lib/workbench'
import { GROUP_WORDS, HELPER_CHIPS, chipCounts, chipWords, finishedToArchive, groupHelpers, taskBanner, type HelperChip } from '~/lib/helperView'
import { edgeWorkerApi, ownCardId, ownTitle, ownWords, type EdgeRun } from '~/lib/edgeWorker'
import { toastUndo } from '~/lib/toast'
import { useThreadStore } from '~/store/threadStore'
import { tn } from '~/strings'
import { useSidebarLayout } from '~/components/sessions/useSidebarLayout'
import { ErrorText } from '~/components/sessions/ErrorText'
import { GithubConnect } from '~/components/sessions/GithubConnect'
import { useSharedConnection } from '~/lib/connection'
import { helpersAway, readSavedHelpers, saveHelpers, type HelpersAway } from '~/lib/helpersSaved'
import { savedLine } from '~/components/SavedStrip'
import { useInHub } from '~/lib/hubWindow'
import { offWhyUnreachable } from '~/lib/failureWords'

const HELPER_EVENTS = ['helpers:change', 'sessions:sync'] as const
const LIST_POLL_MS = 8_000
const DETAIL_POLL_MS = 5_000
/** Each computer is asked over the network, so its lockdown state is read less often. */
const HOSTS_POLL_MS = 60_000
const OWN_POLL_MS = 20_000

interface SessionsSearch {
  id?: string
  task?: string
}

export const Route = createFileRoute('/helpers')({
  validateSearch: (search: Record<string, unknown>): SessionsSearch => {
    const text = (v: unknown) => (typeof v === 'string' && v ? v : undefined)
    const id = text(search.id)
    const task = text(search.task)
    return { ...(id ? { id } : {}), ...(task ? { task } : {}) }
  },
  component: WorkbenchPage,
})

function WorkbenchPage() {
  const { task } = Route.useSearch()
  const selectedId = useParams({ strict: false }).id
  const navigate = Route.useNavigate()
  const [showArchived, setShowArchived] = useState(false)
  const [creating, setCreating] = useState(false)
  const [stopping, setStopping] = useState(false)
  const [sideOpen, setSideOpen] = useState(false)
  const sidebar = useSidebarLayout()
  // Archived agents are always fetched and hidden here, so the toggle is instant and never blanks the list.
  const filters = useMemo(() => ({ ...(task ? { task_id: task } : {}), include_archived: true }), [task])
  const list = usePolled(JSON.stringify(filters), () => globalSessionsApi.list(filters), LIST_POLL_MS, true, true, HELPER_EVENTS)
  const detail = usePolled(selectedId ?? '', () => globalSessionsApi.get(selectedId ?? ''), DETAIL_POLL_MS, Boolean(selectedId), true, HELPER_EVENTS)
  const away = helpersAway(useSharedConnection())
  const inHub = useInHub()
  const offWhy = away ? (away === 'starting' ? t('off_until_started') : offWhyUnreachable(inHub)) : null
  const hosts = usePolled('hosts', () => workbenchApi.hosts(), HOSTS_POLL_MS, true, true, HELPER_EVENTS)
  const afterStop = () => void Promise.all([list.refresh(), hosts.refresh()])

  // An unfiltered list is the nav badge's source, so the nav stops polling while this page is open.
  const setCount = useHQStore((st) => st.setNeedsYouCount)
  const setFeeds = useHQStore((st) => st.setWorkbenchFeedsCount)
  const feeds = !task
  useEffect(() => {
    setFeeds(feeds)
    return () => setFeeds(false)
  }, [feeds, setFeeds])
  useEffect(() => {
    if (!feeds) return
    if (list.error !== null) setCount(0)
    else if (list.data) setCount(needsYouCount(list.data), workingCount(list.data))
  }, [feeds, list.data, list.error, setCount])

  const select = (id: string | undefined) =>
    void navigate({ to: id ? '/helpers/$id' : '/helpers', params: id ? { id } : {}, search: (prev) => ({ ...prev, id: undefined }), replace: true })
  const refreshAll = async () => {
    await Promise.all([list.refresh(), detail.refresh()])
  }
  // The detail fetch is fresher than the list, and also covers an agent the list's filters hide.
  const selected: HarnessSession | null = detail.data ?? list.data?.find((s) => s.id === selectedId) ?? null

  // Folding the list away only makes sense beside an open helper; below 1024 px a missing list would leave nothing to see.
  const folded = sidebar.collapsed && Boolean(selectedId)
  const listPane = folded ? undefined : (
    <div className="relative flex flex-col flex-1 min-h-0">
      <AgentList
        away={away}
        offWhy={offWhy}
        list={list}
        hosts={hosts.data?.hosts ?? []}
        selectedId={selectedId}
        showArchived={showArchived}
        onShowArchived={setShowArchived}
        onNew={() => setCreating(true)}
        onStopAll={() => setStopping(true)}
        onCollapse={selectedId ? sidebar.toggle : undefined}
        task={task}
        onShowAll={() => void navigate({ search: (prev) => ({ ...prev, task: undefined }), replace: true })}
      />
    </div>
  )

  return (
    <>
      <Panes
        list={listPane}
        showDetail={Boolean(selectedId)}
        sideOpen={sideOpen}
        listLabel={t('helpers_list_aria')}
        detailLabel={t('place_helpers')}
        sideLabel={t('helpers_details_aria')}
        detail={
          selectedId ? (
            selected ? (
              <SessionDetail
                key={selected.id}
                session={selected}
                onBack={() => select(undefined)}
                onChanged={refreshAll}
                onToggleSide={() => setSideOpen((o) => !o)}
                onExpandList={folded ? sidebar.toggle : undefined}
              />
            ) : (
              <DetailPlaceholder error={detail.error} loading={detail.loading} onBack={() => select(undefined)} />
            )
          ) : (
            <div className="a-empty">
              <span className="a-empty-ic">
                <Terminal className="a-ic" aria-hidden="true" />
              </span>
              <p className="a-body">{t('helpers_empty_pick')}</p>
            </div>
          )
        }
        side={selected ? <SessionInspector session={selected} onChanged={refreshAll} /> : undefined}
      />
      {creating && (
        <NewAgentDialog
          onClose={() => setCreating(false)}
          onStarted={(id) => {
            setCreating(false)
            select(id)
            void list.refresh()
          }}
        />
      )}
      {stopping && <StopAllDialog onClose={() => setStopping(false)} onStopped={afterStop} />}
    </>
  )
}

interface ListProps {
  away: HelpersAway
  offWhy: string | null
  list: ReturnType<typeof usePolled<HarnessSession[]>>
  hosts: WorkbenchHost[]
  selectedId: string | undefined
  showArchived: boolean
  onShowArchived: (show: boolean) => void
  onNew: () => void
  onStopAll: () => void
  onCollapse?: () => void
  task: string | undefined
  onShowAll: () => void
}

function SectionTitle({ children, count }: { children: string; count: number }) {
  return (
    <p className="a-grp">
      {children}
      <span className="a-count">{count}</span>
    </p>
  )
}

function AgentList({ away, offWhy, list, hosts, selectedId, showArchived, onShowArchived, onNew, onStopAll, onCollapse, task, onShowAll }: ListProps) {
  const inHub = useInHub()
  const [archiveError, setArchiveError] = useState<string | null>(null)
  const [busyId, setBusyId] = useState<string | null>(null)
  const [chip, setChip] = useState<HelperChip>('all')
  const [menuOpen, setMenuOpen] = useState(false)
  const own = usePolled('edge-worker', () => edgeWorkerApi.runs(), OWN_POLL_MS, true, true, HELPER_EVENTS)
  const saved = useMemo(() => (away ? readSavedHelpers() : null), [away])
  const live = away ? null : list.data
  useEffect(() => {
    if (live) saveHelpers(live)
  }, [live])
  const sessions = (away ? saved?.list : list.data) ?? []
  const groups = useMemo(() => groupHelpers(sessions, chip, showArchived), [sessions, chip, showArchived])
  const counts = useMemo(() => chipCounts(sessions), [sessions])
  const banner = task ? taskBanner(sessions, task) : null
  const toArchive = useMemo(() => finishedToArchive(sessions).map((s) => s.id), [sessions])

  const archive = async (ids: string[], archived: boolean) => {
    setBusyId(ids.length === 1 ? ids[0] : 'all')
    setArchiveError(null)
    try {
      await Promise.all(ids.map((id) => globalSessionsApi.archive(id, archived)))
      return true
    } catch {
      setArchiveError(t('helpers_archive_failed'))
      return false
    } finally {
      setBusyId(null)
      await list.refresh()
    }
  }
  // Archiving acts at once; the toast carries the way back.
  const archiveFinished = async () => {
    const ids = toArchive
    if (await archive(ids, true)) {
      toastUndo(tn('helpers_archived_n', ids.length), () => Promise.all(ids.map((id) => globalSessionsApi.archive(id, false))).then(() => list.refresh()), t('helpers_archived_back', { n: ids.length }))
    }
  }

  if (away && !saved) {
    return (
      <div className="a-stale" role="status">
        <TriangleAlert className="a-ic" aria-hidden="true" />
        <span>
          {t('saved_nothing')} {offWhy}
        </span>
      </div>
    )
  }
  if (list.loading && !away) {
    return (
      <div className="a-loading" role="status">
        <span className="a-spin lg" aria-hidden="true" />
        {t('helpers_loading')}
      </div>
    )
  }
  if (!list.data && !away) {
    return <ErrorText className="p-4">{list.error ?? t('helpers_load_failed')}</ErrorText>
  }

  const ownRuns = own.data ?? []
  const showOwn = ownRuns.length > 0 || (own.data !== undefined && sessions.length > 0)
  const shown = sessions.filter((s) => !s.archived).length
  const empty = shown === 0 && ownRuns.length === 0
  const menuItem = (label: string, run: () => void, disabled = false) => (
    <button
      type="button"
      role="menuitem"
      disabled={disabled}
      className="a-menu-item"
      onClick={() => {
        setMenuOpen(false)
        run()
      }}
    >
      {label}
    </button>
  )

  return (
    <>
      <div className="a-pane-head">
        <h2 className="a-h3">{tn('helpers_count', shown)}</h2>
        <button type="button" disabled={away !== null} title={offWhy ?? undefined} onClick={onNew} className="a-btn compact max-[767px]:hidden">
          <Plus className="a-ic" aria-hidden="true" />
          {t('helpers_start_a_helper')}
        </button>
        {onCollapse && (
          <button type="button" onClick={onCollapse} className="a-ib sm max-[1023px]:hidden" aria-label={t('helpers_collapse')}>
            <PanelLeftClose className="a-ic" aria-hidden="true" />
          </button>
        )}
        <div className="relative">
          <button type="button" className="a-ib sm" aria-haspopup="menu" aria-expanded={menuOpen} aria-label={t('helpers_more')} onClick={() => setMenuOpen((o) => !o)}>
            <MoreHorizontal className="a-ic" aria-hidden="true" />
          </button>
          {menuOpen && (
            <div role="menu" className="a-menu absolute right-0 top-full z-10">
              {menuItem(showArchived ? t('helpers_hide_archived') : t('helpers_show_archived'), () => onShowArchived(!showArchived))}
              {menuItem(t('helpers_archive_finished'), () => void archiveFinished(), busyId !== null || toArchive.length === 0)}
              <div className="a-menu-sep" />
              {menuItem(t('stop_all_act'), onStopAll, offWhy !== null)}
            </div>
          )}
        </div>
      </div>
      <div className="flex-1 min-h-0 overflow-y-auto overscroll-contain pb-24">
        {away && saved && (
          <div className="a-stale" role="status">
            <TriangleAlert className="a-ic" aria-hidden="true" />
            <span>
              {savedLine(saved.at, Date.now(), undefined, inHub)} {offWhy}
            </span>
          </div>
        )}
        <LockdownBanner hosts={hosts} />
        <GithubConnect />
        {banner && (
          <div className="a-card mx-4 mb-3" role="status">
            <b className="a-body-b">{banner.title}</b>
            <p className="a-body-sm">{banner.line}</p>
            <button type="button" onClick={onShowAll} className="a-btn secondary compact">
              {t('helpers_show_all')}
            </button>
          </div>
        )}
        <div role="group" aria-label={t('helpers_filters_aria')} className="a-chips px-4 py-2">
          {HELPER_CHIPS.map((c) => (
            <button key={c} type="button" aria-pressed={chip === c} onClick={() => setChip(c)} className={`a-chip ${chip === c ? 'is-on' : ''}`}>
              {t(chipWords(c))}
              <em>{counts[c]}</em>
            </button>
          ))}
        </div>
        {list.error && <ErrorText className="px-4 py-1.5">{t('helpers_refresh_failed', { reason: list.error })}</ErrorText>}
        {archiveError && <ErrorText className="px-4 py-1.5">{archiveError}</ErrorText>}
        {empty && (
          <div className="a-empty">
            <span className="a-empty-ic">
              <Terminal className="a-ic" aria-hidden="true" />
            </span>
            <p className="a-body-b">{t('helpers_empty_title')}</p>
            <p className="a-body-sm a-muted">{t('helpers_empty_body')}</p>
            <button type="button" onClick={onNew} className="a-btn">
              <Plus className="a-ic" aria-hidden="true" />
              {t('helpers_start_a_helper')}
            </button>
          </div>
        )}
        {!empty && groups.length === 0 && chip !== 'all' && <p className="a-body-sm a-muted px-4 py-4">{t('helpers_empty_chip')}</p>}
        {groups.map(({ group, items }) => (
          <section key={group} aria-label={t(GROUP_WORDS[group])} data-group={group}>
            <SectionTitle count={items.length}>{t(GROUP_WORDS[group])}</SectionTitle>
            <ul className="flex flex-col gap-3 px-4">
              {items.map((s) => (
                <SessionRow
                  key={s.id}
                  session={s}
                  selected={s.id === selectedId}
                  onAnswered={() => list.refresh()}
                  onRestore={s.archived ? () => void archive([s.id], false) : undefined}
                  busy={busyId === s.id || busyId === 'all'}
                  answersOff={offWhy}
                />
              ))}
            </ul>
          </section>
        ))}
        {chip === 'all' && showOwn && <OwnWork runs={ownRuns} />}
      </div>
      <button type="button" disabled={away !== null} title={offWhy ?? undefined} onClick={onNew} className="a-fab">
        <Plus className="a-ic" aria-hidden="true" />
        {t('helpers_start_a_helper')}
      </button>
    </>
  )
}

/** What HQ itself is doing for tasks. A card opens its chat; there is no Stop, no Rename and no screen. */
function OwnWork({ runs }: { runs: EdgeRun[] }) {
  const navigate = useNavigate()
  return (
    <section aria-label={t('helpers_own_work')} data-group="own">
      <SectionTitle count={runs.length}>{t('helpers_own_work')}</SectionTitle>
      {runs.length === 0 && <p className="a-body-sm a-muted px-4 py-2">{t('helpers_own_none')}</p>}
      <ul className="flex flex-col gap-3 px-4">
        {runs.map((run) => (
          <li key={ownCardId(run)}>
            <button
              type="button"
              disabled={!run.thread_id}
              onClick={() => {
                if (!run.thread_id) return
                useThreadStore.getState().setActiveThread(run.thread_id)
                void navigate({ to: '/chat' })
              }}
              aria-label={`${ownTitle(run)}, ${ownWords(run)}. ${t('helpers_own_open_chat')}`}
              className="a-card w-full text-left"
            >
              <b className="a-body-b truncate">{ownTitle(run)}</b>
              <span className="a-body-sm a-muted">{ownWords(run)}</span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  )
}

function DetailPlaceholder({ error, loading, onBack }: { error: string | null; loading: boolean; onBack: () => void }) {
  if (loading) {
    return (
      <div className="a-loading" role="status">
        <span className="a-spin lg" aria-hidden="true" />
        {t('helpers_open_loading')}
      </div>
    )
  }
  return (
    <div className="a-empty">
      <ErrorText>{error ?? t('helpers_open_missing')}</ErrorText>
      <button type="button" onClick={onBack} className="a-btn secondary">
        {t('helpers_back')}
      </button>
    </div>
  )
}
