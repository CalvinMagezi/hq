import { useMemo } from 'react'
import { t } from '~/strings'
import { readHqmd, type HqmdNote } from '~/lib/hqmd'
import { readSheet, type Sheet } from '~/lib/sheetCalc'
import { useSuiteEdit } from './useSuiteEdit'
import { SaveLine } from './SaveLine'
import { SheetView } from './SheetView'
import { SlidesView } from './SlidesView'
import { DashboardView } from './DashboardView'
import type { Editor } from './Editor'
import './suite.css'

function sheetsOf(note: HqmdNote): Sheet[] {
  return note.blocks.map(readSheet).filter((s): s is Sheet => s !== null)
}

interface ViewerProps {
  /** The whole note, frontmatter included. */
  text: string
  /** Where it lives. With it a sheet's cells and a card's title and text can be changed; without it the viewer reads. */
  path?: string
  hash?: string
  onSaved?: () => void
}

/** Draws a native sheet, slides or dashboard note. Anything else is the caller's to draw as markdown. */
export function SuiteViewer({ text: source, path, hash, onSaved }: ViewerProps) {
  const saving = useSuiteEdit({ path, content: source, hash, onSaved })
  const text = saving.text
  const editor: Editor = { on: saving.canEdit, apply: saving.apply, path, hash: saving.baseHash, waiting: saving.state === 'queued', reload: () => onSaved?.() }
  const note = useMemo(() => readHqmd(text), [text])
  const sheets = useMemo(() => sheetsOf(note), [note])
  return (
    <div className="su-view">
      {note.fidelity === 'lossy' && <p className="a-body-sm a-muted su-note">{t('suite_fidelity_lossy')}</p>}
      {note.fidelity === 'view' && <p className="a-body-sm a-muted su-note">{t('suite_fidelity_view')}</p>}
      {note.kind === 'sheet' && <SheetView sheets={sheets} editor={editor} />}
      {note.kind === 'slides' && <SlidesView note={note} editor={editor} />}
      {note.kind === 'dashboard' && <DashboardView note={note} sheets={sheets} />}
      <SaveLine state={saving.state} conflict={saving.conflict} onDismiss={saving.dismissConflict} />
    </div>
  )
}
