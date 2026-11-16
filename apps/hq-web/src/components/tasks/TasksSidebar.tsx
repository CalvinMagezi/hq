import { useMemo, useState } from 'react'
import { Archive, ArchiveRestore, ChevronRight, Plus } from 'lucide-react'
import type { Space, Folder, Initiative, TaskItem } from '~/lib/tasksApi'
import { createSpaceClient, createInitiativeClient } from '~/lib/tasksApi'
import { Sheet } from '../ui/Sheet'
import { t } from '~/strings'
import './tasks.css'

export interface TaskSelection {
  spaceId: string | null
  folderId: string | null
  initiativeId: string | null
}

export const ALL_SELECTION: TaskSelection = { spaceId: null, folderId: null, initiativeId: null }

interface Props {
  spaces: Space[]
  folders: Folder[]
  initiatives: Initiative[]
  tasks: TaskItem[]
  selection: TaskSelection
  onSelect: (sel: TaskSelection) => void
  onTaxonomyChanged: () => Promise<void>
  /** Archives a list (hidden here and from the task lists) or brings it back. Its tasks are not touched. */
  onSetListArchived: (initiative: Initiative, archived: boolean) => void
  /** Opens an archived list's tasks in the Archived tab. */
  onOpenArchivedList: (selection: TaskSelection) => void
  onClose: () => void
}

/** A name typed in place: Enter adds it, Escape drops it. */
function AddName({ label, onAdd }: { label: string; onAdd: (name: string) => Promise<void> }) {
  const [open, setOpen] = useState(false)
  const [name, setName] = useState('')
  const [busy, setBusy] = useState(false)
  if (!open) {
    return (
      <button type="button" className="a-btn compact quiet" onClick={() => setOpen(true)}>
        <Plus className="a-ic xs" aria-hidden="true" />
        {label}
      </button>
    )
  }
  const submit = async () => {
    const trimmed = name.trim()
    if (!trimmed || busy) return
    setBusy(true)
    try {
      await onAdd(trimmed)
      setName('')
      setOpen(false)
    } catch (e) {
      console.error('Failed to create:', e)
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="tk-add">
      <input
        autoFocus
        value={name}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') void submit()
          if (e.key === 'Escape') {
            setOpen(false)
            setName('')
          }
        }}
        placeholder={label}
        aria-label={label}
        className="a-input"
      />
      <button type="button" onClick={() => void submit()} disabled={busy || !name.trim()} className="a-btn compact secondary">
        {t('tasks_form_add')}
      </button>
    </div>
  )
}

/** Which list to show: every area with its lists, as cards in a sheet. New areas and lists are typed in place. */
export function TasksSidebar({ spaces, folders, initiatives: allInitiatives, tasks, selection, onSelect, onTaxonomyChanged, onSetListArchived, onOpenArchivedList, onClose }: Props) {
  const initiatives = useMemo(() => allInitiatives.filter((i) => !i.shelved_at), [allInitiatives])
  const archivedLists = useMemo(() => allInitiatives.filter((i) => i.shelved_at), [allInitiatives])
  const [archivedOpen, setArchivedOpen] = useState(false)
  const counts = useMemo(() => {
    const byInitiative = new Map<string, number>()
    for (const task of tasks) byInitiative.set(task.initiative_id, (byInitiative.get(task.initiative_id) ?? 0) + 1)
    return byInitiative
  }, [tasks])
  const count = (id: string) => counts.get(id) ?? 0
  const sum = (list: Initiative[]) => list.reduce((n, i) => n + count(i.id), 0)
  const same = (a: TaskSelection) => a.spaceId === selection.spaceId && a.folderId === selection.folderId && a.initiativeId === selection.initiativeId

  const pick = (sel: TaskSelection) => {
    onSelect(sel)
    onClose()
  }
  const card = (key: string, sel: TaskSelection, name: string, n: number) => (
    <button key={key} type="button" className="a-rcard" aria-pressed={same(sel)} onClick={() => pick(sel)}>
      <div>
        <b>{name}</b>
        <span>{n}</span>
      </div>
    </button>
  )

  return (
    <Sheet title={t('tasks_lists_title')} onClose={onClose} placement="side">
      <div className="tk-tree">
        {card('all', ALL_SELECTION, t('tasks_all_tasks'), tasks.length)}
        {spaces.map((space) => {
          const own = initiatives.filter((i) => i.space_id === space.id)
          return (
            <section key={space.id} className="tk-tree">
              <h3 className="a-grp">{space.name}</h3>
              {card(`s-${space.id}`, { spaceId: space.id, folderId: null, initiativeId: null }, space.name, sum(own))}
              {folders
                .filter((f) => f.space_id === space.id)
                .map((folder) =>
                  card(`f-${folder.id}`, { spaceId: space.id, folderId: folder.id, initiativeId: null }, folder.name, sum(initiatives.filter((i) => i.folder_id === folder.id))),
                )}
              {own.map((i) => (
                <div key={i.id} className="tk-add">
                  {card(`i-${i.id}`, { spaceId: space.id, folderId: i.folder_id ?? null, initiativeId: i.id }, i.name, count(i.id))}
                  <button
                    type="button"
                    className="a-btn compact quiet"
                    onClick={() => onSetListArchived(i, true)}
                    title={t('tasks_archive_list_hint')}
                    aria-label={t('tasks_archive_list_label', { name: i.name })}
                  >
                    <Archive className="a-ic xs" aria-hidden="true" />
                  </button>
                </div>
              ))}
              <AddName
                label={t('tasks_new_list')}
                onAdd={async (name) => {
                  await createInitiativeClient(space.id, name)
                  await onTaxonomyChanged()
                }}
              />
            </section>
          )
        })}
        <AddName
          label={t('tasks_new_area')}
          onAdd={async (name) => {
            await createSpaceClient(name)
            await onTaxonomyChanged()
          }}
        />
        {archivedLists.length > 0 && (
          <section className="tk-tree">
            <button type="button" className="a-btn compact quiet" aria-expanded={archivedOpen} onClick={() => setArchivedOpen((open) => !open)}>
              <ChevronRight className={`a-ic xs transition-transform ${archivedOpen ? 'rotate-90' : ''}`} aria-hidden="true" />
              {t('tasks_archived_lists')} ({archivedLists.length})
            </button>
            {archivedOpen &&
              archivedLists.map((i) => (
                <div key={i.id} className="tk-add">
                  <button
                    type="button"
                    className="a-rcard"
                    onClick={() => {
                      onOpenArchivedList({ spaceId: i.space_id, folderId: i.folder_id ?? null, initiativeId: i.id })
                      onClose()
                    }}
                  >
                    <div>
                      <b>{i.name}</b>
                    </div>
                  </button>
                  <button
                    type="button"
                    className="a-btn compact quiet"
                    onClick={() => onSetListArchived(i, false)}
                    title={t('tasks_unarchive_list_hint')}
                    aria-label={t('tasks_unarchive_list_label', { name: i.name })}
                  >
                    <ArchiveRestore className="a-ic xs" aria-hidden="true" />
                  </button>
                </div>
              ))}
          </section>
        )}
      </div>
    </Sheet>
  )
}
