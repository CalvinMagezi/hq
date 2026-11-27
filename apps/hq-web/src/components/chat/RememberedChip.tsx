import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { Brain } from 'lucide-react'
import { getMemory } from '~/lib/memoryApi'
import { t, tn } from '~/strings'

/** "Remembered 2 notes": what HQ brought up from its memory for this reply. A tap lists the notes, so "why did you say that?" has an answer. */
export function RememberedChip({ ids }: { ids: number[] }) {
  const [open, setOpen] = useState(false)
  if (ids.length === 0) return null
  return (
    <div className="hq-remembered">
      <div className="a-chips" role="group" aria-label={t('chat_remembered_label')}>
        <button type="button" className="a-chip" aria-expanded={open} onClick={() => setOpen(!open)}>
          <Brain className="a-ic xs" aria-hidden="true" />
          {tn('chat_remembered', ids.length)}
        </button>
      </div>
      {open && (
        <div className="a-set" data-testid="remembered-list">
          {ids.map((id) => (
            <RememberedNote key={id} id={id} />
          ))}
          <div className="a-li">
            <Link to="/settings/memory" className="a-btn quiet">
              {t('chat_remembered_manage')}
            </Link>
          </div>
        </div>
      )}
    </div>
  )
}

function RememberedNote({ id }: { id: number }) {
  const note = useQuery({ queryKey: ['memory-one', id], queryFn: () => getMemory(id), retry: false, staleTime: 60_000 })
  if (!note.data && !note.isError) return null
  return (
    <div className="a-li">
      <div className="a-li-main">
        <span>{note.data ? note.data.summary : t('chat_remembered_gone')}</span>
      </div>
    </div>
  )
}
