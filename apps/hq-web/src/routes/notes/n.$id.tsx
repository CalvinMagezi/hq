import { t } from '~/strings'
import { Link, createFileRoute, useNavigate } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { useEffect } from 'react'
import { FileQuestion } from 'lucide-react'
import { notePathById } from '~/lib/noteLink'
import '~/components/vault/notes.css'

// A link by the note's id (Copy link): opens the note wherever it lives now.
export const Route = createFileRoute('/notes/n/$id')({
  component: NoteById,
  ssr: false,
})

function NoteById() {
  const { id } = Route.useParams()
  const navigate = useNavigate()
  const found = useQuery({ queryKey: ['note-by-id', id], queryFn: () => notePathById(id), retry: 1 })
  useEffect(() => {
    if (found.data) navigate({ to: '/notes/$', params: { _splat: found.data }, replace: true })
  }, [found.data, navigate])

  if (found.data === null) {
    return (
      <div className="a-empty">
        <span className="a-empty-ic"><FileQuestion className="a-ic" aria-hidden="true" /></span>
        <b>{t('notes_this_note_is_not')}</b>
        <p className="a-body-sm a-muted">{t('notes_gone_moved_or_deleted')}</p>
        <button type="button" className="a-btn" onClick={() => window.dispatchEvent(new CustomEvent('hq:open-search', { detail: { scope: 'notes' } }))}>
          {t('notes_search_for_it')}
        </button>
        <Link to="/notes" replace className="a-link">
          {t('notes_go_to_notes')}
        </Link>
      </div>
    )
  }
  return (
    <p role="status" className="a-body-sm a-muted" style={{ padding: 24, textAlign: 'center' }}>
      {found.isError ? t('notes_link_failed') : t('notes_link_opening')}
    </p>
  )
}
