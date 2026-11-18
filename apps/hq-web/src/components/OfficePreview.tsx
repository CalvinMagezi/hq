import { useQuery } from '@tanstack/react-query'
import { getPreview } from '~/lib/vaultApi'
import { useAssetUrl } from '~/lib/useAssetUrl'
import { t } from '~/strings'
import { MarkdownViewer } from './MarkdownViewer'

/** A note or an office file as the server reads it: the content, with what was left out said plainly. */
export function OfficePreview({ path }: { path: string }) {
  const fileUrl = useAssetUrl(path)
  const preview = useQuery({ queryKey: ['office-preview', path], queryFn: () => getPreview(path), staleTime: 60_000, retry: false })
  const download = (
    <a className="a-btn secondary compact" href={fileUrl ?? undefined} download>
      {t('office_preview_download')}
    </a>
  )
  if (preview.isPending) return <span role="status" className="a-skel t" aria-label={t('office_preview_loading')} />
  if (preview.isError || !preview.data) {
    return (
      <div role="status" className="a-body-sm a-muted">
        <p>{t('office_preview_failed')}</p>
        {download}
      </div>
    )
  }
  const { banner, notes, markdown, name } = preview.data
  return (
    <div>
      <div className="a-row">
        <b>{name}</b>
        {download}
      </div>
      {banner && <p className="a-body-sm a-muted">{banner}</p>}
      <MarkdownViewer content={markdown} activePath={path} />
      {notes.length > 0 && (
        <div className="a-body-sm a-muted">
          <p>{t('office_preview_left_out')}</p>
          <ul>
            {notes.map((n) => (
              <li key={n}>{n}</li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}
