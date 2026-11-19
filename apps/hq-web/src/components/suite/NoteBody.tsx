import { Suspense, lazy, useMemo } from 'react'
import { t } from '~/strings'
import { hasViewer, readHqmd } from '~/lib/hqmd'
import { extractHtmlDocument } from '~/lib/htmlDashboard'
import { HtmlDashboardView } from '../HtmlDashboardView'
import { MarkdownViewer } from '../MarkdownViewer'

// Each viewer is its own chunk, fetched only when a note of that kind opens, so Chat's first load does not grow.
const SuiteViewer = lazy(() => import('./SuiteViewer').then((m) => ({ default: m.SuiteViewer })))

interface Props {
  /** The whole note, frontmatter included. */
  content: string
  /** Passed on to the markdown viewer when the note is plain. */
  activePath?: string
  zoom?: number
  /** The note's hash, for saving a change against. */
  hash?: string
  /** A change was saved: load the note again. */
  onSaved?: () => void
}

/**
 * A note's body for reading: a native sheet, slides or dashboard gets its viewer, and everything else is
 * markdown, which hqmd allows since it reads without HQ. The note is never changed here.
 */
export function NoteBody({ content, activePath, zoom, hash, onSaved }: Props) {
  const suite = useMemo(() => hasViewer(readHqmd(content)), [content])
  const htmlDoc = useMemo(() => (suite ? null : extractHtmlDocument(content, activePath)), [content, activePath, suite])
  if (htmlDoc) {
    return <HtmlDashboardView html={htmlDoc} path={activePath} />
  }
  if (!suite) {
    return <MarkdownViewer content={content} activePath={activePath} zoom={zoom} />
  }
  return (
    <Suspense fallback={<p className="a-body-sm a-muted">{t('chat_pane_loading')}</p>}>
      <SuiteViewer text={content} path={activePath} hash={hash} onSaved={onSaved} />
    </Suspense>
  )
}
