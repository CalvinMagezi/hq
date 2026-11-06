import { Link } from '@tanstack/react-router'
import { FileText } from 'lucide-react'
import type { PinnedNote, RecentFile } from '~/lib/vaultApi'
import type { SectionState } from '~/lib/homeSections'
import { shortAge } from '~/lib/time'
import { t } from '~/strings'
import { PinnedCard } from './PinnedCard'
import { HomeSection } from './HomeSection'
import { folderTrail, noteName } from './noteRows'
import './notes.css'

interface HomeSectionsProps {
  pinned: SectionState<PinnedNote>
  recent: SectionState<RecentFile>
  onRetryPinned: () => void
  onKeepPin?: (path: string) => void
  onNotNow?: (path: string) => void
  onRetryRecent: () => void
}

/** One recent note as a row: its name, the folder it is in, and how long ago it changed. */
export function RecentRow({ note }: { note: RecentFile }) {
  const trail = folderTrail(note.path)
  return (
    <Link to="/notes/$" params={{ _splat: note.path }} className="a-row">
      <span className="a-av" aria-hidden="true">
        <FileText className="a-ic" />
      </span>
      <span className="a-row-main">
        <b>{note.title || noteName(note.path)}</b>
        <span className="a-body-sm a-muted">
          {trail && <span className="an-trail">{trail}</span>}
          {note.primaryAgent ? ` · ${t('notes_by_hq')}` : ''}
        </span>
      </span>
      <span className="a-body-sm a-muted">{shortAge(note.mtime)}</span>
    </Link>
  )
}

/** Pinned notes as cards, then the notes that changed lately as rows. */
export function HomeSections(p: HomeSectionsProps) {
  return (
    <>
      <HomeSection
        title={t('notes_pinned_hd')}
        state={p.pinned}
        emptyText={t('notes_no_pins')}
        failedKey="fail_load_pinned_no_reply"
        onRetry={p.onRetryPinned}
        render={(notes) => (
          <div className="an-pins">
            {notes.map((note) => (
              <PinnedCard key={note.path} note={note} onKeepPin={p.onKeepPin} onNotNow={p.onNotNow} />
            ))}
          </div>
        )}
      />
      <HomeSection
        title={t('notes_recent')}
        state={p.recent}
        emptyText={t('notes_no_notes_yet')}
        failedKey="fail_load_recent_no_reply"
        onRetry={p.onRetryRecent}
        render={(notes) => (
          <div className="an-rows">
            {notes.map((note) => (
              <RecentRow key={note.path} note={note} />
            ))}
          </div>
        )}
      />
    </>
  )
}
