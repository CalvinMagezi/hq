import { useMemo, useState, useCallback } from 'react'
import { ChevronRight, LayoutGrid, Folder as FolderIcon, FolderOpen, ListChecks, X, Plus, Loader2 } from 'lucide-react'
import type { Space, Folder, Initiative, TaskItem } from '~/lib/tasksApi'
import { createSpaceClient, createFolderClient, createInitiativeClient } from '~/lib/tasksApi'
import { usePersistedState } from '~/lib/usePersistedState'

export interface TaskSelection {
  spaceId: string | null
  folderId: string | null
  initiativeId: string | null
}

interface Props {
  spaces: Space[]
  folders: Folder[]
  initiatives: Initiative[]
  tasks: TaskItem[]
  selection: TaskSelection
  onSelect: (sel: TaskSelection) => void
  onTaxonomyChanged: () => Promise<void>
  mobileOpen: boolean
  onMobileClose: () => void
}

/// Toggles between a small "+" icon and an inline text input (Enter to
/// submit, Escape to cancel) — the same pattern `TaskFormModal`'s "new space"
/// affordance already uses, generalized so Space/Folder/List rows can all add
/// a child without a separate modal for each.
function AddRow({
  placeholder,
  indent,
  alwaysVisible,
  onAdd,
}: {
  placeholder: string
  indent: number
  alwaysVisible?: boolean
  onAdd: (name: string) => Promise<void>
}) {
  const [open, setOpen] = useState(false)
  const [name, setName] = useState('')
  const [busy, setBusy] = useState(false)

  if (!open) {
    return (
      <button
        type="button"
        onClick={() => setOpen(true)}
        title={placeholder}
        className={`p-1 rounded-md text-neutral-600 hover:text-emerald-400 hover:bg-white/5 transition-all shrink-0 ${
          alwaysVisible ? '' : 'opacity-0 group-hover:opacity-100'
        }`}
      >
        <Plus className="w-3 h-3" />
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
    <div className="flex items-center gap-1 py-1" style={{ paddingLeft: `${indent}px` }}>
      <input
        autoFocus
        value={name}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') submit()
          if (e.key === 'Escape') { setOpen(false); setName('') }
        }}
        onBlur={() => { if (!name.trim()) setOpen(false) }}
        placeholder={placeholder}
        className="flex-1 min-w-0 px-2 py-1 rounded-md text-[11px] font-mono text-neutral-200 bg-black/40 border border-white/10 focus:outline-none focus:ring-1 focus:ring-emerald-400"
      />
      <button type="button" onClick={submit} disabled={busy || !name.trim()} className="p-1 text-emerald-400 disabled:opacity-40 shrink-0">
        {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Plus className="w-3 h-3" />}
      </button>
    </div>
  )
}

export const ALL_SELECTION: TaskSelection = { spaceId: null, folderId: null, initiativeId: null }


function rowClass(active: boolean) {
  return `w-full text-left py-1.5 px-2 text-xs font-mono flex items-center gap-1.5 rounded-lg transition-all mx-0.5 group ${
    active ? 'bg-emerald-500/10 text-emerald-400' : 'text-neutral-400 hover:bg-white/5 hover:text-neutral-200'
  }`
}

export function TasksSidebar({ spaces, folders, initiatives, tasks, selection, onSelect, onTaxonomyChanged, mobileOpen, onMobileClose }: Props) {
  const [expandedKeys, setExpandedKeys] = usePersistedState<string[]>('hq-tasks-sidebar-expanded', [], Array.isArray)
  const expanded = useMemo(() => new Set(expandedKeys), [expandedKeys])
  const expand = useCallback(
    (key: string) => setExpandedKeys((prev) => (prev.includes(key) ? prev : [...prev, key])),
    [setExpandedKeys],
  )

  const toggle = useCallback(
    (key: string) =>
      setExpandedKeys((prev) => (prev.includes(key) ? prev.filter((k) => k !== key) : [...prev, key])),
    [setExpandedKeys],
  )

  const counts = useMemo(() => {
    const byInitiative = new Map<string, number>()
    for (const t of tasks) {
      byInitiative.set(t.initiative_id, (byInitiative.get(t.initiative_id) ?? 0) + 1)
    }
    return byInitiative
  }, [tasks])

  const initiativeCount = useCallback(
    (id: string) => counts.get(id) ?? 0,
    [counts]
  )

  const spaceCount = useCallback(
    (spaceId: string) => initiatives.filter((i) => i.space_id === spaceId).reduce((sum, i) => sum + initiativeCount(i.id), 0),
    [initiatives, initiativeCount]
  )

  const folderCount = useCallback(
    (folderId: string) => initiatives.filter((i) => i.folder_id === folderId).reduce((sum, i) => sum + initiativeCount(i.id), 0),
    [initiatives, initiativeCount]
  )

  const isSelected = (sel: Partial<TaskSelection>) =>
    (sel.spaceId ?? null) === selection.spaceId &&
    (sel.folderId ?? null) === selection.folderId &&
    (sel.initiativeId ?? null) === selection.initiativeId

  const body = (
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between px-3 pt-3 pb-2 gap-2">
        <span className="text-[10px] font-mono tracking-widest uppercase font-bold text-neutral-300">Spaces</span>
        <div className="flex items-center gap-1">
          <AddRow
            placeholder="New space"
            indent={0}
            alwaysVisible
            onAdd={async (name) => {
              await createSpaceClient(name)
              await onTaxonomyChanged()
            }}
          />
          <button
            type="button"
            onClick={onMobileClose}
            className="md:hidden p-1 rounded-lg text-neutral-400 hover:text-white hover:bg-white/10"
          >
            <X className="w-4 h-4" />
          </button>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto px-1 pb-4">
        <button
          type="button"
          onClick={() => {
            onSelect(ALL_SELECTION)
            onMobileClose()
          }}
          className={rowClass(isSelected(ALL_SELECTION))}
        >
          <LayoutGrid className="w-3.5 h-3.5 shrink-0" />
          <span className="truncate">All Tasks</span>
          <span className="ml-auto text-[10px] text-neutral-600">{tasks.length}</span>
        </button>

        {spaces.map((space) => {
          const spaceKey = `space:${space.id}`
          const isOpen = expanded.has(spaceKey)
          const spaceFolders = folders.filter((f) => f.space_id === space.id)
          const folderlessInitiatives = initiatives.filter((i) => i.space_id === space.id && !i.folder_id)

          return (
            <div key={space.id} className="mt-1">
              <div className="flex items-center gap-0.5 group">
                <button type="button" onClick={() => toggle(spaceKey)} className="p-1 shrink-0 text-neutral-500 hover:text-neutral-200">
                  <ChevronRight className={`w-3 h-3 transition-transform ${isOpen ? 'rotate-90' : ''}`} />
                </button>
                <button
                  type="button"
                  onClick={() => {
                    onSelect({ spaceId: space.id, folderId: null, initiativeId: null })
                    onMobileClose()
                  }}
                  className={`${rowClass(isSelected({ spaceId: space.id }))} flex-1 min-w-0`}
                >
                  <span className="truncate font-semibold">{space.name}</span>
                  <span className="ml-auto text-[10px] text-neutral-600">{spaceCount(space.id)}</span>
                </button>
                <AddRow
                  placeholder="New list"
                  indent={0}
                  onAdd={async (name) => {
                    await createInitiativeClient(space.id, name)
                    expand(spaceKey)
                    await onTaxonomyChanged()
                  }}
                />
              </div>

              {isOpen && (
                <div className="ml-4 border-l border-white/5 pl-1">
                  {spaceFolders.map((folder) => {
                    const folderKey = `folder:${folder.id}`
                    const folderOpen = expanded.has(folderKey)
                    const folderInitiatives = initiatives.filter((i) => i.folder_id === folder.id)
                    return (
                      <div key={folder.id}>
                        <div className="flex items-center gap-0.5 group">
                          <button
                            type="button"
                            onClick={() => toggle(folderKey)}
                            className="p-1 shrink-0 text-neutral-500 hover:text-neutral-200"
                          >
                            <ChevronRight className={`w-3 h-3 transition-transform ${folderOpen ? 'rotate-90' : ''}`} />
                          </button>
                          <button
                            type="button"
                            onClick={() => {
                              onSelect({ spaceId: space.id, folderId: folder.id, initiativeId: null })
                              onMobileClose()
                            }}
                            className={`${rowClass(isSelected({ spaceId: space.id, folderId: folder.id }))} flex-1 min-w-0`}
                          >
                            {folderOpen ? (
                              <FolderOpen className="w-3.5 h-3.5 shrink-0 text-amber-400/70" />
                            ) : (
                              <FolderIcon className="w-3.5 h-3.5 shrink-0 text-amber-400/70" />
                            )}
                            <span className="truncate">{folder.name}</span>
                            <span className="ml-auto text-[10px] text-neutral-600">{folderCount(folder.id)}</span>
                          </button>
                          <AddRow
                            placeholder="New list"
                            indent={0}
                            onAdd={async (name) => {
                              await createInitiativeClient(space.id, name, folder.name)
                              expand(folderKey)
                              await onTaxonomyChanged()
                            }}
                          />
                        </div>
                        {folderOpen && (
                          <div className="ml-4 border-l border-white/5 pl-1">
                            {folderInitiatives.map((initiative) => (
                              <button
                                key={initiative.id}
                                type="button"
                                onClick={() => {
                                  onSelect({ spaceId: space.id, folderId: folder.id, initiativeId: initiative.id })
                                  onMobileClose()
                                }}
                                className={rowClass(isSelected({ spaceId: space.id, folderId: folder.id, initiativeId: initiative.id }))}
                                style={{ paddingLeft: '22px' }}
                              >
                                <ListChecks className="w-3.5 h-3.5 shrink-0" />
                                <span className="truncate">{initiative.name}</span>
                                <span className="ml-auto text-[10px] text-neutral-600">{initiativeCount(initiative.id)}</span>
                              </button>
                            ))}
                          </div>
                        )}
                      </div>
                    )
                  })}

                  {folderlessInitiatives.map((initiative) => (
                    <button
                      key={initiative.id}
                      type="button"
                      onClick={() => {
                        onSelect({ spaceId: space.id, folderId: null, initiativeId: initiative.id })
                        onMobileClose()
                      }}
                      className={rowClass(isSelected({ spaceId: space.id, initiativeId: initiative.id }))}
                      style={{ paddingLeft: '10px' }}
                    >
                      <ListChecks className="w-3.5 h-3.5 shrink-0" />
                      <span className="truncate">{initiative.name}</span>
                      <span className="ml-auto text-[10px] text-neutral-600">{initiativeCount(initiative.id)}</span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          )
        })}
      </div>
    </div>
  )

  return (
    <>
      {mobileOpen && (
        <div
          className="md:hidden fixed inset-0 z-40 bg-black/60 backdrop-blur-sm"
          onClick={onMobileClose}
        />
      )}
      <aside
        className={`fixed md:relative top-0 left-0 z-50 md:z-auto h-full flex-shrink-0 pad-safe-top pb-[var(--safe-bottom)] md:pb-0 w-[260px] border-r border-white/5 bg-neutral-950 md:bg-black/20 transition-transform duration-200 ${
          mobileOpen ? 'translate-x-0' : '-translate-x-full md:translate-x-0'
        }`}
      >
        {body}
      </aside>
    </>
  )
}
