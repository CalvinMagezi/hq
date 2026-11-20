import { useEffect, useMemo, useRef, useState } from 'react'
import { t } from '~/strings'
import { HqHttpError } from '~/lib/hqAuth'
import { appendBlock, countMatches, imageMarkdown, isImagePath, isOneBlock, outline, relativePath, replaceAll, tableMarkdown } from '~/lib/docEdit'
import { deleteBlock, editBlock, fetchBlocks, insertBlock, type SuiteBlock } from '~/lib/suiteApi'
import { getNote, type DirEntry } from '~/lib/vaultApi'
import { toastAlert, toastNotice, toastUndo } from '~/lib/toast'
import { MarkdownViewer } from '../MarkdownViewer'
import { Sheet } from '../ui/Sheet'
import { SaveLine } from './SaveLine'
import { useSuiteEdit } from './useSuiteEdit'

type Go = { (): Promise<{ blocks: SuiteBlock[] }> }
type Panel = 'outline' | 'find' | 'table' | 'image' | 'blocks' | null

interface Props {
  content: string
  path: string
  hash?: string
  zoom?: number
  onSaved?: () => void
}

/** A doc note: the text, and the tools for a long one: outline, find and replace, a table, a picture, and edit by block. */
export function DocBody({ content, path, hash, zoom, onSaved }: Props) {
  const saving = useSuiteEdit({ path, content, hash, onSaved })
  const [panel, setPanel] = useState<Panel>(null)
  const body = useRef<HTMLDivElement>(null)
  const text = saving.text
  const close = () => setPanel(null)
  const tools: { key: Exclude<Panel, null>; label: string; edit: boolean }[] = [
    { key: 'outline', label: t('suite_doc_outline'), edit: false },
    { key: 'find', label: t('suite_doc_find'), edit: true },
    { key: 'table', label: t('suite_doc_table'), edit: true },
    { key: 'image', label: t('suite_doc_image'), edit: true },
    { key: 'blocks', label: t('suite_doc_blocks'), edit: true },
  ]
  return (
    <div className="su-view su-doc">
      <div className="su-toolbar" role="toolbar" aria-label={t('suite_doc_tools')}>
        {tools
          .filter((x) => !x.edit || saving.canEdit)
          .map((x) => (
            <button key={x.key} type="button" className="a-btn quiet compact" aria-haspopup="dialog" onClick={() => setPanel(x.key)}>
              {x.label}
            </button>
          ))}
      </div>
      <div ref={body}>
        <MarkdownViewer content={text} activePath={path} zoom={zoom} />
      </div>
      <SaveLine state={saving.state} conflict={saving.conflict} onDismiss={saving.dismissConflict} />
      {panel === 'outline' && (
        <OutlinePanel
          text={text}
          onClose={close}
          onPick={(n) => {
            close()
            const heads = body.current?.querySelectorAll('h1,h2,h3,h4,h5,h6')
            setTimeout(() => heads?.[n]?.scrollIntoView({ block: 'start' }), 0)
          }}
        />
      )}
      {panel === 'find' && <FindPanel text={text} apply={saving.apply} onClose={close} />}
      {panel === 'table' && (
        <TablePanel
          onClose={close}
          onInsert={(md) => {
            saving.apply((c) => appendBlock(c, md))
            close()
          }}
        />
      )}
      {panel === 'image' && (
        <ImagePanel
          path={path}
          onClose={close}
          onInsert={(md) => {
            saving.apply((c) => appendBlock(c, md))
            close()
          }}
        />
      )}
      {panel === 'blocks' && <BlockPanel path={path} held={saving.state === 'queued'} onClose={close} onChanged={() => onSaved?.()} />}
    </div>
  )
}

function OutlinePanel({ text, onPick, onClose }: { text: string; onPick: (n: number) => void; onClose: () => void }) {
  const heads = useMemo(() => outline(text), [text])
  return (
    <Sheet title={t('suite_doc_outline')} onClose={onClose} placement="bottom">
      {heads.length === 0 ? (
        <p className="a-body-sm a-muted">{t('suite_doc_no_headings')}</p>
      ) : (
        <nav aria-label={t('suite_doc_outline')}>
          <ul className="su-outline">
            {heads.map((h) => (
              <li key={h.n} style={{ paddingInlineStart: `${(h.level - 1) * 16}px` }}>
                <button type="button" className="su-act" onClick={() => onPick(h.n)}>
                  {h.text}
                </button>
              </li>
            ))}
          </ul>
        </nav>
      )}
    </Sheet>
  )
}

function FindPanel({ text, apply, onClose }: { text: string; apply: (c: (s: string) => string | null) => boolean; onClose: () => void }) {
  const [q, setQ] = useState('')
  const [by, setBy] = useState('')
  const [exact, setExact] = useState(false)
  const n = countMatches(text, q, { caseSensitive: exact })
  return (
    <Sheet
      title={t('suite_doc_find')}
      onClose={onClose}
      placement="bottom"
      footer={
        <button
          type="button"
          className="a-btn compact"
          disabled={n === 0}
          onClick={() => {
            let before = ''
            let after: string | null = null
            const ok = apply((c) => {
              before = c
              after = replaceAll(c, q, by, { caseSensitive: exact })
              return after
            })
            if (!ok) return
            onClose()
            toastUndo(t('suite_doc_replaced', { n: String(n) }), async () => void apply((c) => (c === after ? before : null)), t('suite_undone'))
          }}
        >
          {t('suite_doc_replace_all')}
        </button>
      }
    >
      <div className="su-form">
        <label className="su-field">
          <span className="a-body-sm-b">{t('suite_doc_find_label')}</span>
          <input className="su-select" value={q} onChange={(e) => setQ(e.target.value)} autoFocus />
        </label>
        <label className="su-field">
          <span className="a-body-sm-b">{t('suite_doc_replace_label')}</span>
          <input className="su-select" value={by} onChange={(e) => setBy(e.target.value)} />
        </label>
        <label className="su-check">
          <input type="checkbox" checked={exact} onChange={(e) => setExact(e.target.checked)} />
          <span>{t('suite_doc_match_case')}</span>
        </label>
        <p className="a-body-sm" role="status" aria-live="polite">
          {q === '' ? '' : n === 0 ? t('suite_doc_no_match') : t('suite_doc_matches', { n: String(n) })}
        </p>
      </div>
    </Sheet>
  )
}

function TablePanel({ onInsert, onClose }: { onInsert: (md: string) => void; onClose: () => void }) {
  const [cols, setCols] = useState(3)
  const [rows, setRows] = useState(3)
  return (
    <Sheet
      title={t('suite_doc_table')}
      onClose={onClose}
      placement="bottom"
      footer={
        <button type="button" className="a-btn compact" onClick={() => onInsert(tableMarkdown(cols, rows))}>
          {t('suite_doc_insert')}
        </button>
      }
    >
      <div className="su-form">
        <label className="su-field">
          <span className="a-body-sm-b">{t('suite_doc_table_cols')}</span>
          <input className="su-select" type="number" inputMode="numeric" min={1} max={12} value={cols} onChange={(e) => setCols(Number(e.target.value) || 1)} />
        </label>
        <label className="su-field">
          <span className="a-body-sm-b">{t('suite_doc_table_rows')}</span>
          <input className="su-select" type="number" inputMode="numeric" min={1} max={50} value={rows} onChange={(e) => setRows(Number(e.target.value) || 1)} />
        </label>
        <p className="a-body-sm a-muted">{t('suite_doc_insert_end')}</p>
      </div>
    </Sheet>
  )
}

/** Pick a picture from the vault (its media folder or any folder), write it into the note as markdown. */
function ImagePanel({ path, onInsert, onClose }: { path: string; onInsert: (md: string) => void; onClose: () => void }) {
  const home = path.includes('/') ? path.split('/').slice(0, -1).join('/') : ''
  const [dir, setDir] = useState(home)
  const [entries, setEntries] = useState<DirEntry[] | null>(null)
  const [failed, setFailed] = useState(false)
  const [alt, setAlt] = useState('')
  useEffect(() => {
    let live = true
    setEntries(null)
    setFailed(false)
    getNote(dir)
      .then((r) => live && setEntries(r.dirEntries ?? []))
      .catch(() => live && setFailed(true))
    return () => {
      live = false
    }
  }, [dir])
  const pictures = (entries ?? []).filter((e) => e.isDir || isImagePath(e.path))
  const up = dir.includes('/') ? dir.split('/').slice(0, -1).join('/') : ''
  return (
    <Sheet title={t('suite_doc_image')} onClose={onClose} placement="bottom">
      <div className="su-form">
        <label className="su-field">
          <span className="a-body-sm-b">{t('suite_doc_image_alt')}</span>
          <input className="su-select" value={alt} onChange={(e) => setAlt(e.target.value)} />
        </label>
        <div className="su-actions">
          <button type="button" className="su-act" disabled={dir === ''} onClick={() => setDir(up)}>
            {t('suite_doc_up')}
          </button>
          <button type="button" className="su-act" onClick={() => setDir('_media')}>
            {t('suite_doc_media')}
          </button>
        </div>
        <p className="a-cap a-muted">{dir || t('suite_doc_vault_root')}</p>
        {failed && <p className="a-body-sm a-warn-t">{t('suite_doc_folder_failed')}</p>}
        {entries && pictures.length === 0 && <p className="a-body-sm a-muted">{t('suite_doc_no_images')}</p>}
        <ul className="su-outline">
          {pictures
            .map((e) => (
              <li key={e.path}>
                <button type="button" className="su-act" onClick={() => (e.isDir ? setDir(e.path) : onInsert(imageMarkdown(alt || e.name.replace(/\.[^.]+$/, ''), relativePath(path, e.path))))}>
                  {e.isDir ? `${e.name}/` : e.name}
                </button>
              </li>
            ))}
        </ul>
      </div>
    </Sheet>
  )
}

/** The note as its blocks. A change goes through HQ against the block's hash, so a block that moved on is refused with a sentence. */
function BlockPanel({ path, held, onClose, onChanged }: { path: string; held: boolean; onClose: () => void; onChanged: () => void }) {
  const [blocks, setBlocks] = useState<SuiteBlock[] | null>(null)
  const [open, setOpen] = useState<SuiteBlock | 'end' | null>(null)
  const [after, setAfter] = useState<SuiteBlock | null>(null)
  const [draft, setDraft] = useState('')
  const [message, setMessage] = useState<string | null>(null)
  const load = () => fetchBlocks(path).then((r) => setBlocks(r.blocks)).catch(() => setMessage(t('suite_block_failed')))
  useEffect(() => {
    void load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path])

  const run = async (go: Go) => {
    setMessage(null)
    try {
      const r = await go()
      setBlocks(r.blocks)
      setOpen(null)
      setAfter(null)
      toastNotice(t('suite_edit_saved'))
      onChanged()
    } catch (e) {
      // A block that changed since the list was read is refused: read the list again and say so.
      if (e instanceof HqHttpError && (e.status === 409 || e.status === 422)) setMessage(`${t('suite_block_stale')}${e.message ? ` (${e.message})` : ''}`)
      else toastAlert(t('suite_block_failed'))
      void load()
    }
  }
  const one = isOneBlock(draft)
  return (
    <Sheet title={t('suite_doc_blocks')} onClose={onClose} placement="bottom">
      {held && <p className="a-body-sm a-warn-t">{t('suite_structure_waiting')}</p>}
      {message && (
        <p role="alert" className="a-body-sm a-warn-t">
          {message}
        </p>
      )}
      {open === null ? (
        <>
          <ol className="su-blocks">
            {(blocks ?? []).map((b) => (
              <li key={b.index}>
                <button
                  type="button"
                  className="su-block"
                  disabled={held}
                  onClick={() => {
                    setOpen(b)
                    setDraft(b.src)
                    setMessage(null)
                  }}
                >
                  <span className="a-cap a-muted">{b.kind}</span>
                  <span className="su-block-src">{b.src}</span>
                </button>
                <button
                  type="button"
                  className="a-btn quiet compact"
                  disabled={held}
                  onClick={() => {
                    setOpen('end')
                    setAfter(b)
                    setDraft('')
                  }}
                >
                  {t('suite_block_add_below')}
                </button>
              </li>
            ))}
          </ol>
          <button
            type="button"
            className="a-btn quiet compact"
            disabled={held}
            onClick={() => {
              setOpen('end')
              setAfter(null)
              setDraft('')
            }}
          >
            {t('suite_block_add_end')}
          </button>
        </>
      ) : (
        <div className="su-form">
          <textarea className="su-textarea" rows={8} aria-label={t('suite_block_text')} value={draft} onChange={(e) => setDraft(e.target.value)} />
          {!one && <p className="a-body-sm a-warn-t">{t('suite_block_two')}</p>}
          <div className="su-step">
            <button type="button" className="a-btn quiet compact" onClick={() => setOpen(null)}>
              {t('suite_edit_cancel')}
            </button>
            {open !== 'end' && (
              <button type="button" className="a-btn quiet compact su-danger-t" onClick={() => void run(() => deleteBlock(path, open.index, open.hash))}>
                {t('suite_block_delete')}
              </button>
            )}
            <button
              type="button"
              className="a-btn compact"
              disabled={!one || draft.trim() === ''}
              onClick={() => void run(() => (open === 'end' ? insertBlock(path, draft, after ? { index: after.index, hash: after.hash } : undefined) : editBlock(path, open.index, open.hash, draft)))}
            >
              {t('suite_sheet_save')}
            </button>
          </div>
        </div>
      )}
    </Sheet>
  )
}
