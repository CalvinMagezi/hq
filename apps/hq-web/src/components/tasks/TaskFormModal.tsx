import { useState } from 'react'
import { X, Loader2, Plus } from 'lucide-react'
import type { Folder, Initiative, Space, TaskItem, TaskPriority } from '~/lib/tasksApi'
import { createSpaceClient } from '~/lib/tasksApi'
import { DateRangeInputs, FieldLabel, PRIORITIES } from './taskFields'

interface Props {
  open: boolean
  onClose: () => void
  spaces: Space[]
  folders: Folder[]
  initiatives: Initiative[]
  /** Candidate parents: only top-level tasks can have sub-tasks. */
  tasks: TaskItem[]
  onTaxonomyChanged: () => Promise<void>
  onCreate: (input: {
    title: string
    description: string
    space_id: string
    folder?: string
    initiative: string
    priority?: TaskPriority
    due_date?: string
    start_date?: string
    parent_task_id?: string
    tags: string[]
  }) => Promise<void>
}


export function TaskFormModal({ open, onClose, spaces, folders, initiatives, tasks, onTaxonomyChanged, onCreate }: Props) {
  const [title, setTitle] = useState('')
  const [description, setDescription] = useState('')
  const [spaceId, setSpaceId] = useState(spaces[0]?.id ?? 'personal')
  const [folderName, setFolderName] = useState('')
  const [initiativeName, setInitiativeName] = useState('')
  const [priority, setPriority] = useState<TaskPriority | ''>('')
  const [dueDate, setDueDate] = useState('')
  const [startDate, setStartDate] = useState('')
  const [parentId, setParentId] = useState('')
  const [tags, setTags] = useState('')
  const [submitting, setSubmitting] = useState(false)
  const [addingSpace, setAddingSpace] = useState(false)
  const [newSpaceName, setNewSpaceName] = useState('')

  if (!open) return null

  const foldersForSpace = folders.filter((f) => f.space_id === spaceId)
  const selectedFolder = foldersForSpace.find((f) => f.name.toLowerCase() === folderName.trim().toLowerCase())
  const initiativeNamesForSpace = Array.from(
    new Set(
      initiatives
        .filter((i) => i.space_id === spaceId && i.folder_id === (selectedFolder?.id ?? null))
        .map((i) => i.name)
    )
  )

  const reset = () => {
    setTitle('')
    setDescription('')
    setFolderName('')
    setInitiativeName('')
    setPriority('')
    setDueDate('')
    setStartDate('')
    setParentId('')
    setTags('')
  }

  const parentCandidates = tasks
    .filter((t) => !t.parent_task_id && t.status !== 'complete')
    .sort((a, b) => a.display_id.localeCompare(b.display_id))

  const handleSubmit = async () => {
    if (!title.trim()) return
    setSubmitting(true)
    try {
      await onCreate({
        title: title.trim(),
        description: description.trim(),
        space_id: spaceId,
        folder: folderName.trim() || undefined,
        initiative: initiativeName.trim() || 'Inbox',
        priority: priority || undefined,
        due_date: dueDate || undefined,
        start_date: startDate || undefined,
        parent_task_id: parentId || undefined,
        tags: tags
          .split(',')
          .map((t) => t.trim())
          .filter(Boolean),
      })
      reset()
      onClose()
    } catch (e) {
      console.error('Failed to create task:', e)
    } finally {
      setSubmitting(false)
    }
  }

  const handleAddSpace = async () => {
    if (!newSpaceName.trim()) return
    try {
      const space = await createSpaceClient(newSpaceName.trim())
      setNewSpaceName('')
      setAddingSpace(false)
      await onTaxonomyChanged()
      setSpaceId(space.id)
    } catch (e) {
      console.error('Failed to create space:', e)
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm p-4">
      <div
        className="w-full max-w-lg rounded-2xl border shadow-2xl overflow-hidden"
        style={{ background: 'var(--bg-card, #111418)', borderColor: 'rgba(255,255,255,0.1)' }}
      >
        <div
          className="flex items-center justify-between px-5 py-4 border-b"
          style={{ borderColor: 'rgba(255,255,255,0.08)' }}
        >
          <h2 className="text-sm font-bold text-white">New Task</h2>
          <button type="button" onClick={onClose} className="p-1.5 rounded-lg text-neutral-400 hover:text-white hover:bg-white/10 transition-colors">
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="px-5 py-4 space-y-4 max-h-[70vh] overflow-y-auto">
          <input
            autoFocus
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder="Task title"
            className="w-full px-3.5 py-2.5 rounded-xl text-sm text-neutral-100 hq-field focus:outline-none focus:ring-1 focus:ring-emerald-400"
            style={{ borderColor: 'rgba(255,255,255,0.1)' }}
          />
          <textarea
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="Description (optional)"
            rows={3}
            className="w-full px-3.5 py-2.5 rounded-xl text-xs text-neutral-200 hq-field focus:outline-none focus:ring-1 focus:ring-emerald-400 resize-none"
            style={{ borderColor: 'rgba(255,255,255,0.1)' }}
          />

          <div>
            <FieldLabel>Parent task (optional)</FieldLabel>
            <select
              value={parentId}
              onChange={(e) => setParentId(e.target.value)}
              className="w-full px-2.5 py-1.5 rounded-lg text-xs text-neutral-200 hq-field"
            >
              <option value="">None, a top-level task</option>
              {parentCandidates.map((t) => (
                <option key={t.id} value={t.id}>
                  {t.display_id} {t.title}
                </option>
              ))}
            </select>
            {parentId && (
              <p className="text-[11px] text-neutral-500 mt-1">Filed in the parent's initiative.</p>
            )}
          </div>

          {!parentId && (
            <>
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <FieldLabel>Space</FieldLabel>
                  {addingSpace ? (
                    <div className="flex items-center gap-1.5">
                      <input
                        value={newSpaceName}
                        onChange={(e) => setNewSpaceName(e.target.value)}
                        placeholder="New space name"
                        className="flex-1 px-2.5 py-1.5 rounded-lg text-xs text-neutral-200 bg-black/30 border"
                        style={{ borderColor: 'rgba(255,255,255,0.1)' }}
                      />
                      <button type="button" onClick={handleAddSpace} className="text-xs text-emerald-400 px-1">
                        Add
                      </button>
                    </div>
                  ) : (
                    <div className="flex items-center gap-1.5">
                      <select
                        value={spaceId}
                        onChange={(e) => setSpaceId(e.target.value)}
                        className="flex-1 px-2.5 py-1.5 rounded-lg text-xs text-neutral-200 bg-black/30 border"
                        style={{ borderColor: 'rgba(255,255,255,0.1)' }}
                      >
                        {spaces.map((s) => (
                          <option key={s.id} value={s.id}>
                            {s.name}
                          </option>
                        ))}
                      </select>
                      <button
                        type="button"
                        onClick={() => setAddingSpace(true)}
                        title="New space"
                        className="p-1.5 rounded-lg text-neutral-400 hover:text-emerald-400 hover:bg-white/10"
                      >
                        <Plus className="w-3.5 h-3.5" />
                      </button>
                    </div>
              )}
                </div>

                <div>
                  <FieldLabel>Folder (optional)</FieldLabel>
                  <input
                    list="folder-names"
                    value={folderName}
                    onChange={(e) => setFolderName(e.target.value)}
                    placeholder="None — directly in Space"
                    className="w-full px-2.5 py-1.5 rounded-lg text-xs text-neutral-200 bg-black/30 border"
                    style={{ borderColor: 'rgba(255,255,255,0.1)' }}
                  />
                  <datalist id="folder-names">
                    {foldersForSpace.map((f) => (
                      <option key={f.id} value={f.name} />
                    ))}
                  </datalist>
                </div>
              </div>

              <div>
                <FieldLabel>Initiative</FieldLabel>
                <input
                  list="initiative-names"
                  value={initiativeName}
                  onChange={(e) => setInitiativeName(e.target.value)}
                  placeholder="Inbox"
                  className="w-full px-2.5 py-1.5 rounded-lg text-xs text-neutral-200 bg-black/30 border"
                  style={{ borderColor: 'rgba(255,255,255,0.1)' }}
                />
                <datalist id="initiative-names">
                  {initiativeNamesForSpace.map((name) => (
                    <option key={name} value={name} />
                  ))}
                </datalist>
              </div>
            </>
          )}

          <div className="grid grid-cols-3 gap-3">
            <div>
              <FieldLabel>Priority</FieldLabel>
              <select
                value={priority}
                onChange={(e) => setPriority(e.target.value as TaskPriority | '')}
                className="w-full px-2.5 py-1.5 rounded-lg text-xs text-neutral-200 bg-black/30 border"
                style={{ borderColor: 'rgba(255,255,255,0.1)' }}
              >
                <option value="">None</option>
                {PRIORITIES.map((p) => (
                  <option key={p} value={p}>
                    {p}
                  </option>
                ))}
              </select>
            </div>
            <DateRangeInputs
              start={startDate}
              due={dueDate}
              onStart={setStartDate}
              onDue={setDueDate}
              inputClass="w-full px-2.5 py-1.5 rounded-lg text-xs text-neutral-200 hq-field"
              label={(text) => <FieldLabel>{text}</FieldLabel>}
            />
          </div>

          <div>
            <FieldLabel>Tags (comma-separated; "hq" routes to that agent)</FieldLabel>
            <input
              value={tags}
              onChange={(e) => setTags(e.target.value)}
              placeholder="hq, backend"
              className="w-full px-3 py-2 rounded-lg text-xs text-neutral-200 bg-black/30 border"
              style={{ borderColor: 'rgba(255,255,255,0.1)' }}
            />
          </div>
        </div>

        <div
          className="flex items-center justify-end gap-2 px-5 py-4 border-t"
          style={{ borderColor: 'rgba(255,255,255,0.08)' }}
        >
          <button
            type="button"
            onClick={onClose}
            className="px-4 py-2 rounded-xl text-xs font-semibold text-neutral-400 hover:text-neutral-200 hover:bg-white/5 transition-all"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={handleSubmit}
            disabled={submitting || !title.trim()}
            className="px-5 py-2 rounded-xl text-xs font-bold flex items-center gap-1.5 transition-all disabled:opacity-50"
            style={{ background: 'var(--accent-green, #00ffa3)', color: '#000' }}
          >
            {submitting ? <Loader2 className="w-4 h-4 animate-spin" /> : 'Create Task'}
          </button>
        </div>
      </div>
    </div>
  )
}
