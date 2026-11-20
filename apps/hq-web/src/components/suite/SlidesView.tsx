import { useEffect, useRef, useState } from 'react'
import { t } from '~/strings'
import type { HqmdNote, Slide } from '~/lib/hqmd'
import { setSlideBody, setSlideTitle } from '~/lib/hqmdEdit'
import { LAYOUTS, SPEEDS, TRANSITIONS, addCard, contrastWarning, deleteCard, duplicateCard, moveCard, parseTransition, setCardMeta } from '~/lib/slideEdit'
import { toastUndo } from '~/lib/toast'
import { MarkdownViewer } from '../MarkdownViewer'
import { usePointerDrag } from '../tasks/usePointerDrag'
import type { Editor } from './Editor'
import type { AnchorHint } from '~/lib/commentView'

const SWIPE_PX = 48
const TAP_ZONE = 0.33
function within(v: number, lo: number, hi: number): boolean {
  return lo <= v && v <= hi
}

/** A card's own colours, else the deck's. */
function look(slide: Slide, deck: HqmdNote['deck']) {
  return { background: slide.background ?? deck?.background ?? null, color: slide.color ?? deck?.color ?? null, transition: slide.transition ?? deck?.transition ?? null }
}

const styleOf = (l: { background: string | null; color: string | null }): React.CSSProperties => ({ ...(l.background ? { background: l.background } : {}), ...(l.color ? { color: l.color } : {}) })

/** The first words of a card as plain text, for the small picture of it. */
export function glance(body: string, lines = 3): string {
  return body
    .split('\n')
    .map((l) => l.replace(/^[#>\s*+-]+|^\d+\.\s+/g, '').replace(/[*_`[\]]/g, '').replace(/\]\(.*?\)/g, '').trim())
    .filter(Boolean)
    .slice(0, lines)
    .join(' · ')
}

export function SlidesView({ note, editor, onAnchor }: { note: HqmdNote; editor: Editor; /** Where the person is, for a comment's default block. */ onAnchor?: (hint: AnchorHint) => void }) {
  const [at, setAt] = useState(0)
  const [editingText, setEditingText] = useState(false)
  const [draft, setDraft] = useState('')
  const [presenting, setPresenting] = useState(false)
  const [dragging, setDragging] = useState<{ from: number; over: number } | null>(null)
  const items = useRef<(HTMLLIElement | null)[]>([])
  const count = note.slides.length
  const here = note.slides[Math.min(at, count - 1)]?.title
  useEffect(() => onAnchor?.(here === undefined ? {} : { text: `## ${here}` }), [onAnchor, here])

  const over = (x: number, y: number): number | null => {
    const hit = items.current.findIndex((el) => {
      const r = el?.getBoundingClientRect()
      return !!r && within(x, r.left, r.right) && within(y, r.top, r.bottom)
    })
    return hit < 0 ? null : hit
  }
  const startDrag = usePointerDrag<number>({
    onStart: (from) => setDragging({ from, over: from }),
    onMove: (from, p) => setDragging({ from, over: over(p.x, p.y) ?? from }),
    onEnd: (from, p) => {
      const to = over(p.x, p.y) ?? from
      setDragging(null)
      if (to !== from && editor.apply((c) => moveCard(c, from, to))) setAt(to)
    },
    onCancel: () => setDragging(null),
    onClick: (n) => go(n),
  })

  if (count === 0) {
    return (
      <div className="su-slides-empty">
        <p className="a-body-sm a-muted">{t('suite_no_slides')}</p>
        {editor.on && (
          <button type="button" className="a-btn quiet compact" onClick={() => editor.apply((c) => addCard(c, -1, ''))}>
            {t('suite_card_add')}
          </button>
        )}
      </div>
    )
  }
  const i = Math.min(at, count - 1)
  const slide = note.slides[i]
  const l = look(slide, note.deck)
  const go = (to: number) => {
    setEditingText(false)
    setAt(Math.min(count - 1, Math.max(0, to)))
  }
  const rename = (title: string) => {
    if (title.trim() !== '' && title !== slide.title) editor.apply((current) => setSlideTitle(current, i, slide.title, title))
  }
  const meta = (key: Parameters<typeof setCardMeta>[2], value: string | null) => editor.apply((c) => setCardMeta(c, i, key, value))
  const warn = contrastWarning(l.color, l.background)

  const remove = () => {
    let after: string | null = null
    let before = ''
    const ok = editor.apply((c) => {
      before = c
      after = deleteCard(c, i, slide.title)
      return after
    })
    if (!ok) return
    setAt(Math.max(0, i - 1))
    toastUndo(
      t('suite_card_deleted', { title: slide.title }),
      async () => {
        editor.apply((c) => (c === after ? before : null))
      },
      t('suite_undone'),
    )
  }

  return (
    <div
      className="su-slides"
      tabIndex={0}
      aria-label={t('suite_slides_label')}
      onKeyDown={(e) => {
        // Arrow keys inside a field move its caret, not the slide.
        if ((e.target as HTMLElement).closest('input, textarea, select')) return
        if (e.key === 'ArrowRight' || e.key === 'ArrowDown') {
          e.preventDefault()
          go(i + 1)
        } else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') {
          e.preventDefault()
          go(i - 1)
        }
      }}
    >
      <ol className="su-strip" aria-label={t('suite_slides_label')}>
        {note.slides.map((s, n) => {
          const lk = look(s, note.deck)
          return (
            <li
              key={n}
              ref={(el) => {
                items.current[n] = el
              }}
              className={dragging ? (dragging.from === n ? 'is-dragging' : dragging.over === n ? 'is-over' : undefined) : undefined}
            >
              <button
                type="button"
                className="su-thumb"
                aria-current={n === i ? 'true' : undefined}
                aria-label={t('suite_thumb_label', { n: String(n + 1), title: s.title })}
                onClick={editor.on ? undefined : () => go(n)}
                onPointerDown={editor.on ? (e) => startDrag(e, n) : undefined}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault()
                    go(n)
                  }
                }}
              >
                <span className="su-mini" style={styleOf(lk)}>
                  <b>{s.title}</b>
                  <span>{glance(s.body)}</span>
                </span>
                <span className="a-cap a-muted su-thumb-n">{n + 1}</span>
              </button>
            </li>
          )
        })}
      </ol>
      <section className={`su-stage${slide.layout ? ` su-${slide.layout}` : ''}`} style={styleOf(l)} aria-label={t('suite_slide_n', { n: String(i + 1), total: String(count) })}>
        <div className="su-stage-in">
          {editor.on ? <TitleInput key={`${i}:${slide.title}`} value={slide.title} label={t('suite_edit_title')} onCommit={rename} /> : <h3 className="su-stage-title">{slide.title}</h3>}
          {slide.body && <MarkdownViewer content={slide.body} bare />}
        </div>
      </section>
      {editingText ? (
        <div className="su-body-edit">
          <textarea className="su-textarea" aria-label={t('suite_edit_text_label')} value={draft} rows={8} onChange={(e) => setDraft(e.target.value)} />
          <div className="su-step">
            <button type="button" className="a-btn quiet compact" onClick={() => setEditingText(false)}>
              {t('suite_edit_cancel')}
            </button>
            <button
              type="button"
              className="a-btn secondary compact"
              onClick={() => {
                setEditingText(false)
                editor.apply((current) => setSlideBody(current, i, slide.body, draft))
              }}
            >
              {t('suite_edit_done')}
            </button>
          </div>
        </div>
      ) : (
        <div className="su-step">
          <button type="button" className="a-btn quiet compact" onClick={() => setPresenting(true)}>
            {t('suite_present')}
          </button>
          {editor.on && (
            <button
              type="button"
              className="a-btn quiet compact"
              onClick={() => {
                setDraft(slide.body)
                setEditingText(true)
              }}
            >
              {t('suite_edit_text')}
            </button>
          )}
          <button type="button" className="a-btn quiet compact" disabled={i === 0} onClick={() => go(i - 1)}>
            {t('suite_slide_prev')}
          </button>
          <span className="a-cap a-muted" role="status" aria-live="polite">
            {t('suite_slide_n', { n: String(i + 1), total: String(count) })}
          </span>
          <button type="button" className="a-btn quiet compact" disabled={i === count - 1} onClick={() => go(i + 1)}>
            {t('suite_slide_next')}
          </button>
        </div>
      )}
      {editor.on && (
        <div className="su-card-tools">
          <div className="su-actions" role="group" aria-label={t('suite_card_tools')}>
            <button type="button" className="su-act" onClick={() => editor.apply((c) => addCard(c, i, '')) && setAt(i + 1)}>
              {t('suite_card_add')}
            </button>
            <button type="button" className="su-act" onClick={() => editor.apply((c) => duplicateCard(c, i)) && setAt(i + 1)}>
              {t('suite_card_copy')}
            </button>
            <button type="button" className="su-act" disabled={i === 0} onClick={() => editor.apply((c) => moveCard(c, i, i - 1)) && setAt(i - 1)}>
              {t('suite_card_earlier')}
            </button>
            <button type="button" className="su-act" disabled={i === count - 1} onClick={() => editor.apply((c) => moveCard(c, i, i + 1)) && setAt(i + 1)}>
              {t('suite_card_later')}
            </button>
            <button type="button" className="su-act su-danger-t" onClick={remove}>
              {t('suite_card_delete')}
            </button>
          </div>
          <p className="a-body-sm a-muted su-drag-hint">{t('suite_card_drag_hint')}</p>
          <fieldset className="su-field su-fieldset">
            <legend className="a-body-sm-b">{t('suite_layout')}</legend>
            <div className="su-seg" role="radiogroup" aria-label={t('suite_layout')}>
              {LAYOUTS.map((name) => (
                <button key={name} type="button" role="radio" aria-checked={(slide.layout ?? 'content') === name} className="su-seg-b" onClick={() => meta('layout', name === 'content' ? null : name)}>
                  {t(name === 'content' ? 'suite_layout_content' : 'suite_layout_two')}
                </button>
              ))}
            </div>
          </fieldset>
          <TransitionPicker value={slide.transition} onChange={(v) => meta('transition', v)} />
          <div className="su-colors">
            <ColorField label={t('suite_color_background')} value={slide.background} fallback="#FFFFFF" onChange={(v) => meta('background', v)} />
            <ColorField label={t('suite_color_text')} value={slide.color} fallback="#000000" onChange={(v) => meta('color', v)} />
          </div>
          {warn !== null && (
            <p role="status" className="a-body-sm a-warn-t su-contrast">
              {t('suite_contrast_warn', { ratio: String(warn) })}
            </p>
          )}
          <NotesField key={`${i}:${slide.notes ?? ''}`} value={slide.notes ?? ''} onCommit={(v) => meta('notes', v.trim() === '' ? null : v.trim())} />
        </div>
      )}
      {!editor.on && slide.notes && (
        <details className="su-notes">
          <summary className="a-body-sm-b">{t('suite_notes')}</summary>
          <p className="a-body-sm">{slide.notes}</p>
        </details>
      )}
      {presenting && <Presenter slides={note.slides} deck={note.deck} start={i} onExit={() => setPresenting(false)} />}
    </div>
  )
}

function TransitionPicker({ value, onChange }: { value: string | null; onChange: (v: string | null) => void }) {
  const tr = parseTransition(value) ?? { name: 'none', speed: null }
  const set = (name: string, speed: string | null) => onChange(name === 'none' && !value ? null : speed && name !== 'none' ? `${name} ${speed}` : name)
  return (
    <div className="su-colors">
      <label className="su-field">
        <span className="a-body-sm-b">{t('suite_transition')}</span>
        <select className="su-select" value={tr.name} onChange={(e) => set(e.target.value, tr.speed)}>
          {TRANSITIONS.map((n) => (
            <option key={n} value={n}>
              {n === 'none' ? t('suite_transition_none') : n}
            </option>
          ))}
        </select>
      </label>
      <label className="su-field">
        <span className="a-body-sm-b">{t('suite_transition_speed')}</span>
        <select className="su-select" value={tr.speed ?? ''} disabled={tr.name === 'none'} onChange={(e) => set(tr.name, e.target.value || null)}>
          <option value="">{t('suite_transition_default')}</option>
          {SPEEDS.map((s) => (
            <option key={s} value={s}>
              {s}
            </option>
          ))}
        </select>
      </label>
    </div>
  )
}

function ColorField({ label, value, fallback, onChange }: { label: string; value: string | null; fallback: string; onChange: (v: string | null) => void }) {
  const hex = /^#[0-9a-f]{6}$/i.test(value ?? '') ? (value as string) : fallback
  return (
    <div className="su-field">
      <span className="a-body-sm-b">{label}</span>
      <div className="su-color-row">
        <input type="color" className="su-color" aria-label={label} value={hex} onChange={(e) => onChange(e.target.value.toUpperCase())} />
        <span className="a-body-sm a-muted">{value ?? t('suite_color_theme')}</span>
        {value && (
          <button type="button" className="a-btn quiet compact" onClick={() => onChange(null)}>
            {t('suite_color_clear')}
          </button>
        )}
      </div>
    </div>
  )
}

function NotesField({ value, onCommit }: { value: string; onCommit: (v: string) => void }) {
  const [text, setText] = useState(value)
  return (
    <label className="su-field">
      <span className="a-body-sm-b">{t('suite_notes')}</span>
      <textarea className="su-textarea" rows={3} value={text} placeholder={t('suite_notes_placeholder')} onChange={(e) => setText(e.target.value)} onBlur={() => text !== value && onCommit(text)} />
    </label>
  )
}

/** A card's title, written in place: it is kept when the person leaves the field or presses Enter. */
function TitleInput({ value, label, onCommit }: { value: string; label: string; onCommit: (value: string) => void }) {
  const [text, setText] = useState(value)
  useEffect(() => setText(value), [value])
  return (
    <input
      className="su-title-input su-stage-title"
      aria-label={label}
      value={text}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => onCommit(text)}
      onKeyDown={(e) => {
        if (e.key === 'Enter') e.currentTarget.blur()
        else if (e.key === 'Escape') {
          setText(value)
          e.currentTarget.blur()
        }
      }}
    />
  )
}

/**
 * A slide at full window: Arrow keys, Space and Page keys move, Escape leaves; a swipe or a tap on the left
 * or right third of the screen moves too. It asks the browser for full screen and works without it. Nothing is saved.
 */
function Presenter({ slides, deck, start, onExit }: { slides: Slide[]; deck: HqmdNote['deck']; start: number; onExit: () => void }) {
  const [at, setAt] = useState(start)
  const [notes, setNotes] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  const down = useRef<{ x: number; y: number } | null>(null)
  const step = (d: number) => setAt((n) => Math.min(slides.length - 1, Math.max(0, n + d)))

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onExit()
      else if (['ArrowRight', 'ArrowDown', ' ', 'PageDown'].includes(e.key)) step(1)
      else if (['ArrowLeft', 'ArrowUp', 'PageUp'].includes(e.key)) step(-1)
      else return
      e.preventDefault()
      e.stopPropagation()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [onExit])

  // Full screen where the browser allows it; leaving it by the browser's own gesture ends the show.
  useEffect(() => {
    const el = root.current
    let entered = false
    try {
      void el?.requestFullscreen?.().then(
        () => {
          entered = true
        },
        () => undefined,
      )
    } catch {
      // No Fullscreen API: the dialog still fills the window.
    }
    const left = () => {
      if (entered && !document.fullscreenElement) onExit()
    }
    document.addEventListener('fullscreenchange', left)
    return () => {
      document.removeEventListener('fullscreenchange', left)
      if (document.fullscreenElement) void document.exitFullscreen().catch(() => undefined)
    }
  }, [onExit])

  const slide = slides[at]
  const l = look(slide, deck)
  const tr = parseTransition(l.transition)
  return (
    <div
      ref={root}
      className="su-present"
      role="dialog"
      aria-modal="true"
      aria-label={t('suite_present_label')}
      style={styleOf(l)}
      onPointerDown={(e) => {
        down.current = { x: e.clientX, y: e.clientY }
      }}
      onPointerUp={(e) => {
        const d = down.current
        down.current = null
        if (!d || (e.target as HTMLElement).closest('button')) return
        const dx = e.clientX - d.x
        const dy = e.clientY - d.y
        if (SWIPE_PX <= Math.abs(dx) && Math.abs(dy) < Math.abs(dx)) return step(dx < 0 ? 1 : -1)
        if (Math.abs(dx) > 10) return
        const w = e.currentTarget.getBoundingClientRect()
        const f = (e.clientX - w.left) / w.width
        if (f < TAP_ZONE) step(-1)
        else if (f > 1 - TAP_ZONE) step(1)
      }}
    >
      <div className="su-present-bar">
        <button type="button" className="a-btn quiet compact" onClick={() => setNotes((n) => !n)} aria-pressed={notes} disabled={!slide.notes}>
          {t('suite_notes')}
        </button>
        <button type="button" className="a-btn quiet compact" onClick={onExit}>
          {t('suite_present_exit')}
        </button>
      </div>
      <div key={at} className={`su-present-card${tr && tr.name !== 'none' ? ` su-tr-${tr.name} su-sp-${tr.speed ?? 'medium'}` : ''}${slide.layout ? ` su-${slide.layout}` : ''}`}>
        <h2 className="su-present-title">{slide.title}</h2>
        {slide.body && <MarkdownViewer content={slide.body} bare />}
      </div>
      {notes && slide.notes && <p className="su-present-notes a-body-sm">{slide.notes}</p>}
      <p className="a-cap su-present-n" role="status" aria-live="polite">
        {t('suite_slide_n', { n: String(at + 1), total: String(slides.length) })}
      </p>
    </div>
  )
}
