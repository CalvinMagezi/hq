import { useEffect, useId, useRef, useState } from 'react'
import { AlertCircle } from 'lucide-react'
import { Sheet } from '~/components/ui/Sheet'
import { t, type StringKey } from '~/strings'
import type { BudgetAction, BudgetDef, BudgetPeriod } from '~/lib/usageApi'
import {
  PERIOD_KEY,
  WORK_KEY,
  WORK_KINDS,
  canSaveLimit,
  defFromForm,
  emptyLimitForm,
  formFromDef,
  limitErrors,
  serviceName,
  type LimitForm,
  type ScopeKind,
} from '~/lib/spendLimits'

const KINDS: { kind: ScopeKind; title: StringKey; sub: StringKey }[] = [
  { kind: 'global', title: 'spend_cover_all', sub: 'spend_cover_all_sub' },
  { kind: 'provider', title: 'spend_cover_service', sub: 'spend_cover_service_sub' },
  { kind: 'model', title: 'spend_cover_model', sub: 'spend_cover_model_sub' },
  { kind: 'origin', title: 'spend_cover_work', sub: 'spend_cover_work_sub' },
]

const ACTIONS: { action: BudgetAction; title: StringKey; sub: StringKey }[] = [
  { action: 'block', title: 'spend_act_block', sub: 'spend_act_block_sub' },
  { action: 'downgrade', title: 'spend_act_downgrade', sub: 'spend_act_downgrade_sub' },
  { action: 'notify', title: 'spend_act_notify', sub: 'spend_act_notify_sub' },
]

const PERIODS: BudgetPeriod[] = ['day', 'week', 'month']

function Msg({ id, text }: { id: string; text: string }) {
  return (
    <p id={id} className="a-field-msg a-warn-t flex items-center gap-1.5" role="alert">
      <AlertCircle className="a-ic" aria-hidden="true" />
      {text}
    </p>
  )
}

interface Props {
  /** The limit being edited, or null for a new one. */
  editing: BudgetDef | null
  /** Every limit now, so a name already in use can say it will be replaced. */
  existing: BudgetDef[]
  /** The ids of the AI services HQ knows, for "One AI service". */
  services: string[]
  /** Model ids seen in the spending, offered while typing "One model". */
  models: string[]
  busy: boolean
  failed: boolean
  resetLine: string
  onSave: (def: BudgetDef) => void
  onClose: () => void
}

/** One sheet for Add and Edit, in choices instead of config fields. */
export function LimitSheet({ editing, existing, services, models, busy, failed, resetLine, onSave, onClose }: Props) {
  const [form, setForm] = useState<LimitForm>(() => (editing ? formFromDef(editing) : emptyLimitForm()))
  const [dirty, setDirty] = useState<Set<string>>(new Set())
  const nameRef = useRef<HTMLInputElement>(null)
  const errors = limitErrors(form)
  const set = (patch: Partial<LimitForm>) => {
    setForm((f) => ({ ...f, ...patch }))
    setDirty((d) => new Set([...d, ...Object.keys(patch)]))
  }
  const clash = existing.some((d) => d.name === form.name.trim() && d.name !== editing?.name)
  const show = (k: keyof typeof errors) => {
    const key = errors[k]
    return dirty.has(k) && key ? <Msg id={`limit-err-${k}`} text={t(key)} /> : null
  }

  const formId = useId()

  useEffect(() => {
    nameRef.current?.focus()
  }, [])

  return (
    <Sheet
      title={t(editing ? 'spend_sheet_edit' : 'spend_sheet_add')}
      placement="side"
      busy={busy}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="a-btn secondary" onClick={onClose}>{t('spend_cancel')}</button>
          <button type="submit" form={formId} className="a-btn" disabled={busy || !canSaveLimit(form)}>
            {busy ? t('spend_saving') : t('spend_save')}
          </button>
        </>
      }
    >
      <form
        id={formId}
        className="flex flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault()
          if (canSaveLimit(form) && !busy) onSave(defFromForm(form))
        }}
      >
        <div className="a-field">
          <label className="a-field-label" htmlFor="limit-name">{t('spend_name')}</label>
          <input id="limit-name" ref={nameRef} className="a-input" value={form.name} onChange={(e) => set({ name: e.target.value })} />
          {show('name')}
          {clash && <Msg id="limit-err-replace" text={t('spend_err_replace', { name: form.name.trim() })} />}
        </div>

        <div role="radiogroup" aria-label={t('spend_covers')} className="flex flex-col gap-2">
          <span className="a-field-label">{t('spend_covers')}</span>
          {KINDS.map((k) => (
            <button
              key={k.kind}
              type="button"
              role="radio"
              aria-checked={form.kind === k.kind}
              className="a-rcard"
              onClick={() => setForm((f) => ({ ...f, kind: k.kind, value: '' }))}
            >
              <i className="a-radio" aria-hidden="true" />
              <div>
                <b>{t(k.title)}</b>
                <span>{t(k.sub)}</span>
              </div>
            </button>
          ))}
        </div>

        {form.kind === 'provider' && (
          <div className="a-field">
            <label className="a-field-label" htmlFor="limit-service">{t('spend_pick_service')}</label>
            <select id="limit-service" className="a-input" value={form.value} onChange={(e) => set({ value: e.target.value })}>
              <option value="" />
              {services.map((s) => (
                <option key={s} value={s}>{serviceName(s)}</option>
              ))}
            </select>
            {show('value')}
          </div>
        )}
        {form.kind === 'model' && (
          <div className="a-field">
            <label className="a-field-label" htmlFor="limit-model">{t('spend_pick_model')}</label>
            <input id="limit-model" className="a-input" list="limit-models" value={form.value} onChange={(e) => set({ value: e.target.value })} />
            <datalist id="limit-models">
              {models.map((m) => (
                <option key={m} value={m} />
              ))}
            </datalist>
            {show('value')}
          </div>
        )}
        {form.kind === 'origin' && (
          <div className="a-field">
            <label className="a-field-label" htmlFor="limit-work">{t('spend_pick_work')}</label>
            <select id="limit-work" className="a-input" value={form.value} onChange={(e) => set({ value: e.target.value })}>
              <option value="" />
              {WORK_KINDS.map((w) => (
                <option key={w} value={w}>{t(WORK_KEY[w])}</option>
              ))}
            </select>
            {show('value')}
          </div>
        )}

        <div className="a-field">
          <span className="a-field-label">{t('spend_how_often')}</span>
          <div className="a-seg" role="group" aria-label={t('spend_how_often')}>
            {PERIODS.map((p) => (
              <button key={p} type="button" className="a-seg-item" aria-pressed={form.period === p} onClick={() => set({ period: p })}>
                {t(PERIOD_KEY[p])}
              </button>
            ))}
          </div>
          <p className="a-field-msg">{resetLine}</p>
        </div>

        <div className="a-field">
          <label className="a-field-label" htmlFor="limit-usd">{t('spend_limit_dollars')}</label>
          <input id="limit-usd" className="a-input" inputMode="decimal" value={form.limit} onChange={(e) => set({ limit: e.target.value })} />
          {show('limit')}
        </div>

        <div className="a-field">
          <label className="a-field-label" htmlFor="limit-warn">{t('spend_warn_at')}</label>
          <input id="limit-warn" className="a-input" inputMode="numeric" value={form.warn} onChange={(e) => set({ warn: e.target.value })} />
          <p className="a-field-msg">{t('spend_warn_at_pct')}</p>
          {show('warn')}
        </div>

        <div role="radiogroup" aria-label={t('spend_at_the_limit')} className="flex flex-col gap-2">
          <span className="a-field-label">{t('spend_at_the_limit')}</span>
          {ACTIONS.map((a) => (
            <button key={a.action} type="button" role="radio" aria-checked={form.action === a.action} className="a-rcard" onClick={() => set({ action: a.action })}>
              <i className="a-radio" aria-hidden="true" />
              <div>
                <b>{t(a.title)}</b>
                <span>{t(a.sub)}</span>
              </div>
            </button>
          ))}
        </div>
        {form.action === 'downgrade' && (
          <div className="a-field">
            <label className="a-field-label" htmlFor="limit-cheaper">{t('spend_cheaper_model')}</label>
            <input id="limit-cheaper" className="a-input" list="limit-models" value={form.downgrade} onChange={(e) => set({ downgrade: e.target.value })} />
            {show('downgrade')}
          </div>
        )}

        <p className="a-field-msg">{t('spend_applies')}</p>
        {failed && <Msg id="limit-err-save" text={t('spend_save_failed')} />}
      </form>
    </Sheet>
  )
}
