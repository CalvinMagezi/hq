import { useQuery } from '@tanstack/react-query'
import { FileText, Globe, File as FileIcon } from 'lucide-react'
import { getNote } from '~/lib/vaultApi'
import { readHqmd } from '~/lib/hqmd'
import { computeSheet, formatValue, readSheet } from '~/lib/sheetCalc'
import { t } from '~/strings'
import type { MadeNote, ReplySource, SourceKind } from '~/lib/replySources'
import { folderLabel } from '~/lib/workPane'

const KIND_ICON: Record<SourceKind, typeof FileText> = { note: FileText, file: FileIcon, web: Globe }

export function SourceIcon({ kind }: { kind: SourceKind }) {
  const Icon = KIND_ICON[kind]
  return <Icon className="a-ic xs" aria-hidden="true" />
}

/** What a reply read, one chip each. A click shows it in the pane's Context tab, not in a new browser tab. */
export function SourceChips({ sources, onOpen }: { sources: ReplySource[]; onOpen: (source: ReplySource) => void }) {
  if (sources.length === 0) return null
  return (
    <div className="a-chips hq-sources" role="group" aria-label={t('chat_sources_label')}>
      {sources.map((s) => (
        <button key={s.ref} type="button" className="a-chip" aria-label={t('chat_source_open', { title: s.title })} onClick={() => onOpen(s)}>
          <SourceIcon kind={s.kind} />
          {s.title}
        </button>
      ))}
    </div>
  )
}

/** Something HQ made in this reply: kind, title, where it was saved, and Open, which shows it in the pane. */
export function MadeCard({ note, onOpen }: { note: MadeNote; onOpen: (note: MadeNote) => void }) {
  const folder = folderLabel(note.path)
  return (
    <div className="hq-made hq-made-col">
      <div className="hq-made-row">
      <FileText className="a-ic" aria-hidden="true" />
      <span className="hq-made-main">
        <b>{note.title}</b>
        <span className="a-cap a-muted">{t('chat_made_saved', { folder })}</span>
      </span>
      <button type="button" className="a-btn secondary compact" onClick={() => onOpen(note)}>
        {t('chat_made_open')}
      </button>
      </div>
      <SheetPreview path={note.path} />
    </div>
  )
}

const PREVIEW_ROWS = 3

/** The first rows of a sheet a reply made, so a person sees what it is before opening it. Other notes show nothing here. */
function SheetPreview({ path }: { path: string }) {
  const note = useQuery({ queryKey: ['pane-note', path], queryFn: () => getNote(path), staleTime: 60_000, retry: false })
  const content = note.data?.content
  const sheet = content ? readHqmd(content).blocks.map(readSheet).find((x) => x !== null) : null
  if (!content || !sheet || readHqmd(content).kind !== 'sheet') return null
  const cells = computeSheet(sheet)
  const total = sheet.totalRows ?? sheet.rows.length
  const more = total > PREVIEW_ROWS
  return (
    <div className="hq-made-preview">
      <table className="su-preview">
        <thead>
          <tr>
            {sheet.header.map((h) => (
              <th key={h}>{h}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {sheet.rows.slice(0, PREVIEW_ROWS).map((row, r) => (
            <tr key={r}>
              {row.map((raw, c) => {
                const cell = cells[r]?.[c]
                return <td key={c}>{cell?.ok ? formatValue(cell.value, sheet.types[c]) : raw}</td>
              })}
            </tr>
          ))}
        </tbody>
      </table>
      {more && <p className="a-cap a-muted">{t('suite_preview_rows', { shown: String(PREVIEW_ROWS), total: String(total) })}</p>}
    </div>
  )
}
