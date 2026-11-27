import { useState } from 'react'
import { Dialog } from '~/components/ui/Dialog'
import {
  MEMORY_FILTERS,
  memoryBadges,
  recallModeKey,
  standingAction,
  type Done,
  type MemoryFilter,
  type MemoryItem,
  type MemoryList,
  type Standing,
} from '~/lib/memoryApi'
import { dateByAge } from '~/lib/time'
import { t, tn } from '~/strings'

/** What the owner is shown after forgetting a note: what HQ could not reach. */
export interface ForgottenNote {
  /** Words for what is still there, from the catalogue. */
  stillThere: Array<'memory_still_chat' | 'memory_still_backups'>
}

export interface MemoryViewProps {
  list: MemoryList
  filter: MemoryFilter
  query: string
  canChange: boolean
  /** The note a change is running for. */
  busyId: number | null
  error: string | null
  forgotten: ForgottenNote | null
  onFilter: (filter: MemoryFilter) => void
  onQuery: (query: string) => void
  /** Each answers whether it worked, so the view knows when to close what it opened. */
  onSave: (id: number, summary: string) => Done
  onPin: (id: number, pinned: boolean) => void
  onStanding: (id: number, standing: Standing) => void
  onForget: (id: number) => Done
  /** Whether HQ saves new notes. Undefined until HQ answers, and the switch is then off the table. */
  learning?: boolean
  onLearning?: (on: boolean) => void
  nowMs?: number
  /** For tests: start with a note's options open, its editor open, or its Forget question asked. */
  initial?: { openId?: number; editingId?: number; confirmId?: number }
}

export function memoryMeta(item: MemoryItem, nowMs: number): string {
  return [
    dateByAge(item.created_at, nowMs),
    ...memoryBadges(item).map((key) => t(key)),
    item.used > 0 ? tn('memory_used', item.used) : '',
  ]
    .filter(Boolean)
    .join(' · ')
}

function hintFor(item: MemoryItem): string | null {
  if (item.origin !== 'owner') return t('memory_hint_shared')
  if (item.status === 'review') return t('memory_hint_review')
  if (item.status === 'archived') return t('memory_hint_archived')
  return null
}

/** Settings > What HQ remembers: the notes HQ saved from the owner's chats, with a way to correct, hide or delete each. */
export function MemoryView({
  list,
  filter,
  query,
  canChange,
  busyId,
  error,
  forgotten,
  onFilter,
  onQuery,
  onSave,
  onPin,
  onStanding,
  onForget,
  learning,
  onLearning,
  nowMs = Date.now(),
  initial,
}: MemoryViewProps) {
  const [openId, setOpenId] = useState<number | null>(initial?.openId ?? null)
  const [editingId, setEditingId] = useState<number | null>(initial?.editingId ?? null)
  const [draft, setDraft] = useState('')
  const [draftError, setDraftError] = useState<string | null>(null)
  const [confirmId, setConfirmId] = useState<number | null>(initial?.confirmId ?? null)
  const [forgetting, setForgetting] = useState(false)

  const modeKey = recallModeKey(list.recall)
  const { counts } = list

  const startEdit = (item: MemoryItem) => {
    setEditingId(item.id)
    setDraft(item.summary)
    setDraftError(null)
  }
  const save = async (id: number) => {
    if (!draft.trim()) {
      setDraftError(t('memory_edit_empty'))
      return
    }
    setDraftError(null)
    if (await onSave(id, draft)) setEditingId(null)
  }
  const forget = async (id: number) => {
    setForgetting(true)
    const ok = await onForget(id)
    setForgetting(false)
    if (ok) {
      setConfirmId(null)
      setOpenId(null)
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <p className="a-body-sm a-muted">{t('memory_intro')}</p>
      {modeKey && (
        <p className="a-body-sm a-muted" data-testid="memory-mode">
          {t(modeKey)}
        </p>
      )}
      <p className="a-body-sm" data-testid="memory-counts">
        {t('memory_counts', {
          total: counts.total,
          active: counts.active,
          review: counts.review,
          outdated: counts.out_of_date,
          archived: counts.archived,
        })}
      </p>
      {onLearning && (
        <div className="a-set" data-testid="memory-learning">
          <div className="a-li">
            <div className="a-li-main">
              <b id="memory-learning-label">{t('memory_learning_item')}</b>
              <span>{learning === false ? t('memory_learning_paused') : t('memory_learning_on')}</span>
            </div>
            <button
              type="button"
              role="switch"
              className="a-sw"
              aria-checked={learning === true}
              aria-labelledby="memory-learning-label"
              disabled={learning === undefined || !canChange}
              onClick={() => onLearning(learning !== true)}
            />
          </div>
        </div>
      )}
      {!canChange && (
        <p className="a-body-sm a-warn-t" data-testid="memory-offline">
          {t('memory_offline')}
        </p>
      )}
      {error && (
        <p role="alert" className="a-body-sm a-warn-t">
          {error}
        </p>
      )}
      {forgotten && (
        <div className="a-body-sm" role="status" data-testid="memory-forgotten">
          <p>{t('memory_forgotten')}</p>
          {forgotten.stillThere.length > 0 && (
            <>
              <p className="a-muted">{t('memory_still_there')}</p>
              <ul>
                {forgotten.stillThere.map((key) => (
                  <li key={key}>{t(key)}</li>
                ))}
              </ul>
            </>
          )}
        </div>
      )}

      <div className="a-field">
        <label className="a-field-label" htmlFor="memory-search">
          {t('memory_search_label')}
        </label>
        <input
          id="memory-search"
          type="search"
          className="a-input"
          placeholder={t('memory_search_placeholder')}
          value={query}
          onChange={(e) => onQuery(e.target.value)}
        />
      </div>

      <div className="a-chips" role="group" aria-label={t('memory_filter_label')}>
        {MEMORY_FILTERS.map((f) => (
          <button
            key={f.id}
            type="button"
            className="a-chip"
            aria-pressed={filter === f.id && !query.trim()}
            onClick={() => onFilter(f.id)}
          >
            {t(f.label)}
          </button>
        ))}
      </div>

      {list.items.length === 0 ? (
        <p className="a-body-sm a-muted" data-testid="memory-empty">
          {filter === 'all' && !query.trim() ? t('memory_empty') : t('memory_empty_filter')}
        </p>
      ) : (
        <div className="a-set">
          {list.items.map((item) => {
            const open = openId === item.id
            const editing = editingId === item.id
            const busy = busyId === item.id
            const hint = hintFor(item)
            return (
              <div key={item.id} className="a-li" data-testid="memory-row" aria-busy={busy || undefined}>
                <div className="a-li-main">
                  {editing ? (
                    <>
                      <label className="a-field-label" htmlFor={`memory-edit-${item.id}`}>
                        {t('memory_edit_label')}
                      </label>
                      <textarea
                        id={`memory-edit-${item.id}`}
                        className="a-input"
                        rows={3}
                        value={draft}
                        onChange={(e) => setDraft(e.target.value)}
                      />
                      {draftError && (
                        <span role="alert" className="a-warn-t">
                          {draftError}
                        </span>
                      )}
                      <div className="flex flex-wrap gap-2">
                        <button type="button" className="a-btn" disabled={busy} onClick={() => void save(item.id)}>
                          {busy ? t('memory_saving') : t('memory_save')}
                        </button>
                        <button type="button" className="a-btn secondary" disabled={busy} onClick={() => setEditingId(null)}>
                          {t('memory_cancel')}
                        </button>
                      </div>
                    </>
                  ) : (
                    <b>{item.summary}</b>
                  )}
                  <span>{memoryMeta(item, nowMs)}</span>
                  {hint && <span>{hint}</span>}
                  {open && !editing && (
                    <div className="flex flex-wrap gap-2" data-testid="memory-options">
                      <button type="button" className="a-btn secondary" disabled={!canChange || busy} onClick={() => startEdit(item)}>
                        {t('memory_edit')}
                      </button>
                      <button
                        type="button"
                        className="a-btn secondary"
                        disabled={!canChange || busy}
                        onClick={() => onPin(item.id, !item.pinned)}
                      >
                        {item.pinned ? t('memory_unpin') : t('memory_pin')}
                      </button>
                      <button
                        type="button"
                        className="a-btn secondary"
                        disabled={!canChange || busy}
                        onClick={() => onStanding(item.id, standingAction(item).to)}
                      >
                        {t(standingAction(item).label)}
                      </button>
                      <button
                        type="button"
                        className="a-btn quiet danger"
                        disabled={!canChange || busy}
                        onClick={() => setConfirmId(item.id)}
                      >
                        {t('memory_forget')}
                      </button>
                    </div>
                  )}
                </div>
                {!editing && (
                  <button
                    type="button"
                    className="a-btn quiet"
                    aria-expanded={open}
                    onClick={() => setOpenId(open ? null : item.id)}
                  >
                    {t('memory_options')}
                  </button>
                )}
              </div>
            )
          })}
        </div>
      )}

      {confirmId !== null && (
        <Dialog
          kind="red"
          title={t('memory_forget_title')}
          body={t('memory_forget_body')}
          keepLabel={t('memory_forget_keep')}
          actLabel={t('memory_forget_act')}
          busyLabel={t('memory_forget_busy')}
          busy={forgetting}
          error={null}
          onKeep={() => setConfirmId(null)}
          onAct={() => void forget(confirmId)}
        />
      )}
    </div>
  )
}
