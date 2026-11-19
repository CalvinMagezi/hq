import { useState } from 'react'
import { Download, Maximize2 } from 'lucide-react'
import { t } from '~/strings'
import { htmlSaveFileName } from '~/lib/htmlDashboard'
import { Sheet } from './ui/Sheet'

interface Props {
  html: string
  path?: string | null
  title?: string
}

/** Saves an HTML document string directly from memory without sending any auth token. */
export function saveHtmlAsFile(html: string, path?: string | null): void {
  const blob = new Blob([html], { type: 'text/html;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = htmlSaveFileName(path)
  document.body.appendChild(a)
  a.click()
  a.remove()
  setTimeout(() => URL.revokeObjectURL(url), 10_000)
}

/**
 * Renders a standalone HTML dashboard in a sandboxed iframe (`allow-scripts` only,
 * without `allow-same-origin`, and never receiving the web token), with Open full screen
 * and Save as file actions.
 */
export function HtmlDashboardView({ html, path, title }: Props) {
  const [fullscreen, setFullscreen] = useState(false)
  const label = title?.trim() || path?.split('/').filter(Boolean).pop()?.replace(/\.[^.]+$/, '') || t('html_dashboard_label')

  const saveButton = (
    <button type="button" className="a-btn secondary compact" onClick={() => saveHtmlAsFile(html, path)}>
      <Download className="a-ic xs" aria-hidden="true" />
      {t('html_save_file')}
    </button>
  )

  return (
    <div className="hq-html-dash">
      <div className="hq-html-bar">
        <button type="button" className="a-btn secondary compact" onClick={() => setFullscreen(true)}>
          <Maximize2 className="a-ic xs" aria-hidden="true" />
          {t('html_open_fullscreen')}
        </button>
        {saveButton}
      </div>
      <iframe title={label} srcDoc={html} sandbox="allow-scripts" className="hq-html-frame" />
      {fullscreen && (
        <Sheet
          title={label}
          placement="center"
          className="hq-html-fullscreen"
          onClose={() => setFullscreen(false)}
          headActions={saveButton}
        >
          <iframe title={label} srcDoc={html} sandbox="allow-scripts" className="hq-html-frame hq-html-frame-full" />
        </Sheet>
      )}
    </div>
  )
}
