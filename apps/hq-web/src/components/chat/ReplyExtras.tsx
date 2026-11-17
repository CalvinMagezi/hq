import { FileText, Globe, File as FileIcon } from 'lucide-react'
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
    <div className="hq-made">
      <FileText className="a-ic" aria-hidden="true" />
      <span className="hq-made-main">
        <b>{note.title}</b>
        <span className="a-cap a-muted">{t('chat_made_saved', { folder })}</span>
      </span>
      <button type="button" className="a-btn secondary compact" onClick={() => onOpen(note)}>
        {t('chat_made_open')}
      </button>
    </div>
  )
}
