import { t } from '~/strings'
import { Link, createFileRoute, useNavigate } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { useEffect } from 'react'
import { notePathById } from '~/lib/noteLink'

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
      <div className="h-full flex flex-col items-center justify-center gap-3 p-6 text-center">
        <h1 className="text-sm font-bold" style={{ color: 'var(--text-primary)' }}>{t('notes_this_note_is_not')}</h1>
        <p className="text-xs" style={{ color: 'var(--text-dim)' }}>{t('notes_gone_moved_or_deleted')}</p>
        <Link to="/notes" replace className="min-h-11 px-4 inline-flex items-center rounded-lg text-xs font-bold" style={{ color: 'var(--accent-blue)' }}>
          {t('notes_go_to_notes')}
        </Link>
      </div>
    )
  }
  return (
    <div role="status" className="h-full flex items-center justify-center text-xs" style={{ color: 'var(--text-dim)' }}>
      {found.isError ? t('notes_link_failed') : t('notes_link_opening')}
    </div>
  )
}
