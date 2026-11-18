import { useState, type ReactNode } from 'react'
import { Link } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { ExternalLink, ListChecks, MessageSquare, Maximize2, Pencil, Pin, PinOff, X } from 'lucide-react'
import { t, tn } from '~/strings'
import type { ToolStep } from '~/store/threadStore'
import { formatCredits, totalCredits, type StepCredit } from '~/lib/stepCredits'
import { sourcesOf, type ReplySource } from '~/lib/replySources'
import { secondsRunning } from '~/lib/liveLine'
import { stripFrontmatter } from '~/lib/noteConflict'
import { hasViewer, readHqmd } from '~/lib/hqmd'
import { getNote, togglePinNote } from '~/lib/vaultApi'
import type { PaneState, PaneTab } from '~/lib/workPane'
import { MarkdownViewer } from '../MarkdownViewer'
import { NoteBody } from '../suite/NoteBody'
import { SourceIcon } from './ReplyExtras'

const TABS: { id: PaneTab; label: 'chat_pane_result' | 'chat_pane_context' | 'chat_pane_helpers' }[] = [
  { id: 'result', label: 'chat_pane_result' },
  { id: 'context', label: 'chat_pane_context' },
  { id: 'helpers', label: 'chat_pane_helpers' },
]

const KIND_LABEL = { note: 'chat_kind_note', file: 'chat_kind_file', web: 'chat_kind_web' } as const
// A preview in the Context tab shows the start of a note, not all of it.
const PREVIEW_CHARS = 900

interface Props {
  pane: PaneState
  onTab: (tab: PaneTab) => void
  onClose: () => void
  /** What the reply so far has done: the Context tab reads it. */
  steps: ToolStep[]
  startedAt?: number
  credits?: StepCredit[]
  running: boolean
  onSelectSource: (ref: string | null) => void
  onAsk: (path: string) => void
  onTask: (path: string) => void
  /** The Helpers tab: the sessions this chat watches. */
  helpers: ReactNode
}

/** The pane beside a chat: what HQ made (Result), what it is leaning on (Context), and the helpers it watches. */
export function WorkPane({ pane, onTab, onClose, steps, startedAt, credits, running, onSelectSource, onAsk, onTask, helpers }: Props) {
  return (
    <>
      <div className="a-pane-head hq-pane-head">
        <div role="tablist" aria-label={t('chat_pane_tabs')} className="hq-pane-tabs">
          {TABS.map((tab) => (
            <button
              key={tab.id}
              type="button"
              role="tab"
              id={`hq-pane-tab-${tab.id}`}
              aria-selected={pane.tab === tab.id}
              aria-controls="hq-pane-panel"
              tabIndex={pane.tab === tab.id ? 0 : -1}
              className="hq-pane-tab"
              onClick={() => onTab(tab.id)}
              onKeyDown={(e) => {
                const at = TABS.findIndex((x) => x.id === pane.tab)
                const next = e.key === 'ArrowRight' ? at + 1 : e.key === 'ArrowLeft' ? at - 1 : null
                if (next === null) return
                e.preventDefault()
                const to = TABS[(next + TABS.length) % TABS.length].id
                onTab(to)
                requestAnimationFrame(() => document.getElementById(`hq-pane-tab-${to}`)?.focus())
              }}
            >
              {t(tab.label)}
            </button>
          ))}
        </div>
        <button type="button" className="a-ib sm" aria-label={t('close')} onClick={onClose}>
          <X className="a-ic sm" aria-hidden="true" />
        </button>
      </div>
      <div id="hq-pane-panel" role="tabpanel" aria-labelledby={`hq-pane-tab-${pane.tab}`} className="hq-pane-body">
        {pane.tab === 'result' && <ResultTab path={pane.note} onAsk={onAsk} onTask={onTask} />}
        {pane.tab === 'context' && (
          <ContextTab steps={steps} startedAt={startedAt} credits={credits} running={running} selected={pane.source} onSelect={onSelectSource} />
        )}
        {pane.tab === 'helpers' && helpers}
      </div>
    </>
  )
}

function noteTitle(path: string): string {
  return (path.split('/').filter(Boolean).pop() ?? path).replace(/\.md$/, '')
}

function ResultTab({ path, onAsk, onTask }: { path: string | null; onAsk: (path: string) => void; onTask: (path: string) => void }) {
  if (!path) {
    return (
      <div className="hq-pane-empty">
        <p className="a-body-b">{t('chat_pane_empty_title')}</p>
        <p className="a-body-sm a-muted">{t('chat_pane_empty_line')}</p>
      </div>
    )
  }
  return <NoteResult key={path} path={path} onAsk={onAsk} onTask={onTask} />
}

function NoteResult({ path, onAsk, onTask }: { path: string; onAsk: (path: string) => void; onTask: (path: string) => void }) {
  const note = useQuery({ queryKey: ['pane-note', path], queryFn: () => getNote(path), retry: false })
  const content = note.data?.content ?? ''
  const pinnedNow = /^---[\s\S]*?^pinned:\s*true/m.test(content)
  const [pinned, setPinned] = useState<boolean | null>(null)
  const isPinned = pinned ?? pinnedNow
  const missing = note.isError || (note.data && !note.data.isDir && content === '')
  return (
    <div className="hq-result">
      <div className="hq-result-head">
        <h3 className="a-h3">{noteTitle(path)}</h3>
        <Link to="/notes/$" params={{ _splat: path }} className="a-ib sm" aria-label={t('chat_pane_open_vault')} title={t('chat_pane_open_vault')}>
          <Maximize2 className="a-ic sm" aria-hidden="true" />
        </Link>
      </div>
      <div className="a-chips an-bar" role="group" aria-label={t('notes_actions')}>
        <Link to="/notes/$" params={{ _splat: path }} className="a-btn compact">
          <Pencil className="a-ic" aria-hidden="true" />
          {t('note_editor_edit')}
        </Link>
        <button type="button" className="a-chip" onClick={() => onAsk(path)}>
          <MessageSquare className="a-ic xs" aria-hidden="true" />
          {t('notes_ask_hq')}
        </button>
        <button type="button" className="a-chip" onClick={() => onTask(path)}>
          <ListChecks className="a-ic xs" aria-hidden="true" />
          {t('notes_make_task')}
        </button>
        <button
          type="button"
          className="a-chip"
          aria-pressed={isPinned}
          onClick={() => {
            const next = !isPinned
            setPinned(next)
            void togglePinNote(path, next).catch(() => setPinned(!next))
          }}
        >
          {isPinned ? <PinOff className="a-ic xs" aria-hidden="true" /> : <Pin className="a-ic xs" aria-hidden="true" />}
          {isPinned ? t('pin_unpin') : t('notes_pin')}
        </button>
      </div>
      {note.isPending && <p className="a-body-sm a-muted">{t('chat_pane_loading')}</p>}
      {missing && <p role="alert" className="a-body-sm a-muted">{t('chat_pane_missing')}</p>}
      {!missing && content !== '' && <ResultBody content={content} path={path} hash={note.data?.hash} onSaved={() => void note.refetch()} />}
    </div>
  )
}

/** The note as the pane shows it: a suite viewer for a sheet, slides or dashboard, else its text. */
function ResultBody({ content, path, hash, onSaved }: { content: string; path: string; hash?: string; onSaved: () => void }) {
  const suite = hasViewer(readHqmd(content))
  return suite ? <NoteBody content={content} activePath={path} hash={hash} onSaved={onSaved} /> : <MarkdownViewer content={stripFrontmatter(content)} bare />
}

interface ContextProps {
  steps: ToolStep[]
  startedAt?: number
  credits?: StepCredit[]
  running: boolean
  selected: string | null
  onSelect: (ref: string | null) => void
}

function ContextTab({ steps, startedAt, credits, running, selected, onSelect }: ContextProps) {
  const sources = sourcesOf(steps)
  const reading = sources.filter((s) => s.reading)
  const used = sources.filter((s) => !s.reading && s.kind === 'note')
  const others = sources.filter((s) => !s.reading && s.kind !== 'note')
  const total = totalCredits(credits)
  const chosen = sources.find((s) => s.ref === selected) ?? null
  const row = (s: ReplySource) => (
    <li key={s.ref}>
      <button type="button" className="hq-ctx-row" aria-current={s.ref === selected ? 'true' : undefined} onClick={() => onSelect(s.ref === selected ? null : s.ref)}>
        <SourceIcon kind={s.kind} />
        <span className="hq-ctx-main">
          <b>{s.title}</b>
          <span className="a-cap a-muted">{t(KIND_LABEL[s.kind])}</span>
        </span>
      </button>
    </li>
  )
  return (
    <div className="hq-ctx">
      {reading.length > 0 && (
        <section>
          <h3 className="a-grp">{t('chat_ctx_reading')}</h3>
          <ul className="hq-ctx-list">{reading.map(row)}</ul>
        </section>
      )}
      {used.length > 0 && (
        <section>
          <h3 className="a-grp">{t('chat_ctx_used')}</h3>
          <ul className="hq-ctx-list">{used.map(row)}</ul>
        </section>
      )}
      <section>
        <h3 className="a-grp">{t('chat_ctx_sources')}</h3>
        {others.length > 0 ? <ul className="hq-ctx-list">{others.map(row)}</ul> : <p className="a-body-sm a-muted hq-ctx-none">{t('chat_ctx_none')}</p>}
      </section>
      {chosen && <SourceDetail source={chosen} />}
      <section>
        <h3 className="a-grp">{t('chat_ctx_reply')}</h3>
        <dl className="hq-ctx-facts">
          <div>
            <dt className="a-cap a-muted">{tn('chat_ctx_steps', steps.length)}</dt>
          </div>
          {running && <div><dt className="a-cap a-muted">{t('chat_ctx_seconds', { n: String(secondsRunning(startedAt, Date.now())) })}</dt></div>}
          {total !== null && <div><dt className="a-cap a-muted">{formatCredits(total)}</dt></div>}
        </dl>
      </section>
    </div>
  )
}

function SourceDetail({ source }: { source: ReplySource }) {
  const note = useQuery({
    queryKey: ['pane-note', source.ref],
    queryFn: () => getNote(source.ref),
    enabled: source.kind === 'note',
    retry: false,
  })
  const text = note.data ? stripFrontmatter(note.data.content).trim().slice(0, PREVIEW_CHARS) : ''
  return (
    <section className="hq-ctx-detail" aria-label={source.title}>
      <div className="hq-ctx-detail-head">
        <b>{source.title}</b>
        {source.kind === 'note' && (
          <Link to="/notes/$" params={{ _splat: source.ref }} className="a-ib sm" aria-label={t('chat_pane_open_vault')}>
            <Maximize2 className="a-ic sm" aria-hidden="true" />
          </Link>
        )}
        {source.kind === 'web' && (
          <a href={source.ref} target="_blank" rel="noopener noreferrer" className="a-btn secondary compact">
            <ExternalLink className="a-ic xs" aria-hidden="true" />
            {t('chat_ctx_open_page')}
          </a>
        )}
      </div>
      {source.kind === 'file' && <p className="a-cap a-muted">{source.ref}</p>}
      {text && <MarkdownViewer content={text} bare />}
    </section>
  )
}
