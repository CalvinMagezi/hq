import { t } from '~/strings'
import { useState, type KeyboardEvent } from 'react'
import { Plus, X } from 'lucide-react'
import { Sheet } from '../ui/Sheet'
import './tasks.css'
import type { Folder, Initiative, Space, TaskItem, TaskPriority } from '~/lib/tasksApi'
import { createSpaceClient } from '~/lib/tasksApi'
import { DateRangeInputs, EstimateInput, FieldLabel, PRIORITIES, isInvalidEstimate, parseEstimate } from './taskFields'

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
    estimate_minutes?: number
    parent_task_id?: string
    tags: string[]
    assignees?: string[]
  }) => Promise<void>
}


/** Names and tags: what is understood turns into a chip, and a tap on the chip takes it out. */
function ChipField({ chips, onChange, placeholder, label }: { chips: string[]; onChange: (next: string[]) => void; placeholder: string; label: string }) {
  const [draft, setDraft] = useState('')
  const commit = () => {
    const words = draft.split(',').map((w) => w.trim()).filter((w) => w && !chips.includes(w))
    if (words.length) onChange([...chips, ...words])
    setDraft('')
  }
  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter' || e.key === ',') {
      e.preventDefault()
      commit()
    } else if (e.key === 'Backspace' && draft === '' && chips.length > 0) onChange(chips.slice(0, -1))
  }
  return (
    <div className="tk-tagf">
      {chips.map((c) => (
        <span key={c} className="tk-tag">
          {c}
          <button type="button" className="a-ib" aria-label={t('tasks_remove_chip', { label: c })} onClick={() => onChange(chips.filter((x) => x !== c))}>
            <X className="a-ic xs" aria-hidden="true" />
          </button>
        </span>
      ))}
      <input value={draft} onChange={(e) => setDraft(e.target.value)} onKeyDown={onKey} onBlur={commit} placeholder={placeholder} aria-label={label} />
    </div>
  )
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
  const [estimate, setEstimate] = useState('')
  const [parentId, setParentId] = useState('')
  const [tags, setTags] = useState<string[]>([])
  const [assignees, setAssignees] = useState<string[]>([])
  const [details, setDetails] = useState(false)
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
    setEstimate('')
    setParentId('')
    setTags([])
    setAssignees([])
    setDetails(false)
  }

  const parentCandidates = tasks
    .filter((t) => !t.parent_task_id && t.status !== 'complete')
    .sort((a, b) => a.display_id.localeCompare(b.display_id))

  const handleSubmit = async () => {
    if (!title.trim() || isInvalidEstimate(estimate)) return
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
        estimate_minutes: parseEstimate(estimate) ?? undefined,
        parent_task_id: parentId || undefined,
        tags,
        assignees,
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

  const canSubmit = !submitting && title.trim() !== '' && !isInvalidEstimate(estimate)

  return (
    <Sheet
      title={t('tasks_new_task')}
      onClose={onClose}
      busy={submitting}
      footer={
        <>
          <button type="button" onClick={onClose} className="a-btn secondary">
            {t('dl_btn_cancel')}
          </button>
          <button type="button" onClick={handleSubmit} disabled={!canSubmit} className="a-btn">
            {submitting ? t('tasks_adding') : t('tasks_add_task')}
          </button>
        </>
      }
    >
      <div className="tk-stack">
        <input
          autoFocus
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && canSubmit && !details) void handleSubmit()
          }}
          placeholder={t('tasks_what_needs_doing')}
          aria-label={t('tasks_form_title')}
          className="a-input"
        />
        {!details && (
          <button type="button" className="a-btn compact quiet" onClick={() => setDetails(true)}>
            {t('tasks_add_details')}
          </button>
        )}
        {details && (
          <>
            <textarea value={description} onChange={(e) => setDescription(e.target.value)} placeholder={t('tasks_form_description')} rows={3} className="a-input" />

            <div>
              <FieldLabel>{t('tasks_form_parent')}</FieldLabel>
              <select value={parentId} onChange={(e) => setParentId(e.target.value)} className="a-input">
                <option value="">{t('tasks_form_parent_none')}</option>
                {parentCandidates.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.display_id} {p.title}
                  </option>
                ))}
              </select>
              {parentId && <p className="a-body-sm a-muted">{t('tasks_filed_in_parent_list')}</p>}
            </div>

            {!parentId && (
              <>
                <div className="tk-two">
                  <div>
                    <FieldLabel>{t('tasks_field_area')}</FieldLabel>
                    {addingSpace ? (
                      <div className="tk-add">
                        <input value={newSpaceName} onChange={(e) => setNewSpaceName(e.target.value)} placeholder={t('tasks_new_area_name')} className="a-input" />
                        <button type="button" onClick={handleAddSpace} className="a-btn compact secondary">
                          {t('tasks_form_add')}
                        </button>
                      </div>
                    ) : (
                      <div className="tk-add">
                        <select value={spaceId} onChange={(e) => setSpaceId(e.target.value)} className="a-input">
                          {spaces.map((s) => (
                            <option key={s.id} value={s.id}>
                              {s.name}
                            </option>
                          ))}
                        </select>
                        <button type="button" onClick={() => setAddingSpace(true)} aria-label={t('tasks_new_area')} className="a-ib">
                          <Plus className="a-ic" aria-hidden="true" />
                        </button>
                      </div>
                    )}
                  </div>
                  <div>
                    <FieldLabel>{t('tasks_form_folder')}</FieldLabel>
                    <input list="folder-names" value={folderName} onChange={(e) => setFolderName(e.target.value)} placeholder={t('tasks_none_in_area')} className="a-input" />
                    <datalist id="folder-names">
                      {foldersForSpace.map((f) => (
                        <option key={f.id} value={f.name} />
                      ))}
                    </datalist>
                  </div>
                </div>
                <div>
                  <FieldLabel>{t('tasks_field_list')}</FieldLabel>
                  <input list="initiative-names" value={initiativeName} onChange={(e) => setInitiativeName(e.target.value)} placeholder={t('tasks_form_inbox')} className="a-input" />
                  <datalist id="initiative-names">
                    {initiativeNamesForSpace.map((name) => (
                      <option key={name} value={name} />
                    ))}
                  </datalist>
                </div>
              </>
            )}

            <div>
              <FieldLabel>{t('tasks_priority')}</FieldLabel>
              <select value={priority} onChange={(e) => setPriority(e.target.value as TaskPriority | '')} className="a-input">
                <option value="">{t('tasks_priority_none')}</option>
                {PRIORITIES.map((p) => (
                  <option key={p} value={p}>
                    {t(`tasks_priority_${p}`)}
                  </option>
                ))}
              </select>
            </div>
            <div className="tk-two">
              <DateRangeInputs start={startDate} due={dueDate} onStart={setStartDate} onDue={setDueDate} inputClass="a-input" label={(text) => <FieldLabel>{text}</FieldLabel>} />
            </div>
            <EstimateInput value={estimate} onChange={setEstimate} inputClass="a-input" label={(text) => <FieldLabel>{text}</FieldLabel>} />

            <div>
              <FieldLabel>{t('task_drawer_assigned_label')}</FieldLabel>
              <ChipField chips={assignees} onChange={setAssignees} placeholder={t('task_drawer_assigned_hint')} label={t('task_drawer_assigned_label')} />
            </div>
            <div>
              <FieldLabel>{t('task_drawer_tags_label')}</FieldLabel>
              <ChipField chips={tags} onChange={setTags} placeholder={t('task_drawer_tags_hint')} label={t('task_drawer_tags_label')} />
              <p className="a-body-sm a-muted">{t('tasks_words_hq_understood_are')}</p>
            </div>
          </>
        )}
      </div>
    </Sheet>
  )
}
