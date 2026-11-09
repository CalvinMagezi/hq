import { Link } from '@tanstack/react-router'
import { Pin } from 'lucide-react'
import type { PinnedNote } from '~/lib/vaultApi'
import { relTime } from '~/lib/time'
import { t } from '~/strings'
import { folderTrail } from './noteRows'
import './notes.css'

interface PinnedCardProps {
  note: PinnedNote
  onKeepPin?: (path: string) => void
  onNotNow?: (path: string) => void
}

/** A pinned note as a card: title, a line of what is in it, where it lives. A suggested pin says who and asks. */
export function PinnedCard({ note, onKeepPin, onNotNow }: PinnedCardProps) {
  const trail = folderTrail(note.path)
  const suggested = Boolean(note.suggestedBy)
  return (
    <div className={`a-card an-pin${suggested ? ' an-sug' : ''}`}>
      <Link to="/notes/$" params={{ _splat: note.path }} className="an-pin-link">
        <b>{note.title}</b>
        {note.preview && <span className="a-body-sm a-muted">{note.preview}</span>}
        <span className="a-body-sm a-muted">
          {!suggested && <Pin className="a-ic" aria-hidden="true" />} {[trail, relTime(note.updatedAt)].filter(Boolean).join(' · ')}
        </span>
      </Link>
      {suggested && (
        <div className="an-sug-foot">
          <span className="a-body-sm a-muted">{t('pin_suggested_by', { name: note.suggestedBy ?? '' })}</span>
          <div className="a-btns">
            {onKeepPin && (
              <button type="button" className="a-btn" onClick={() => onKeepPin(note.path)}>
                {t('pin_keep')}
              </button>
            )}
            {onNotNow && (
              <button type="button" className="a-btn secondary" onClick={() => onNotNow(note.path)}>
                {t('pin_not_now')}
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  )
}
