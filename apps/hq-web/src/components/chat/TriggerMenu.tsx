import { useEffect, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { FileText, Folder, ListChecks, Terminal } from 'lucide-react'
import { t } from '~/strings'
import { useHQStore } from '~/store/hqStore'
import { globalSessionsApi } from '~/lib/sessionsApi'
import { getNoteTree, getPinnedNotes, searchNotes } from '~/lib/vaultApi'
import { matchFolders } from '~/lib/palette'
import { matchHelpers, matchTasks, SEARCH_DEBOUNCE_MS } from '~/lib/search'
import { sessionTitle } from '~/lib/workbench'
import { agentName, computerName } from '~/lib/names'
import { STATUS_LABELS } from '~/lib/tasksApi'
import { folderReference, helperReference, noteReference, taskReference, topLevelFolders, type Trigger } from '~/lib/triggers'

export type TriggerKind = 'notes' | 'tasks' | 'helpers' | 'folders'

export interface TriggerItem {
  id: string
  kind: TriggerKind
  title: string
  sub: string
  /** What goes in the text when this is picked. */
  reference: string
}

const GROUP_KEY = {
  notes: 'chat_trigger_notes',
  tasks: 'chat_trigger_tasks',
  helpers: 'chat_trigger_helpers',
  folders: 'chat_trigger_folders',
} as const
const ICON = { notes: FileText, tasks: ListChecks, helpers: Terminal, folders: Folder } as const
// A few of each kind keep the list short enough to read at a glance.
const PER_GROUP = 4

/** What a trigger offers: "@" notes, tasks and helpers the person already has; "#" folders. */
export function useTriggerItems(trigger: Trigger | null): TriggerItem[] {
  const char = trigger?.char
  const query = trigger?.query ?? ''
  const tasks = useHQStore((s) => s.tasks)
  // The search waits for a pause in typing, as the palette does.
  const [settled, setSettled] = useState(query)
  useEffect(() => {
    const timer = window.setTimeout(() => setSettled(query), SEARCH_DEBOUNCE_MS)
    return () => window.clearTimeout(timer)
  }, [query])
  const typed = settled.trim().length > 0

  const found = useQuery({ queryKey: ['trigger', 'notes', settled], queryFn: () => searchNotes(settled), enabled: char === '@' && typed, staleTime: 30_000 })
  const pinned = useQuery({ queryKey: ['trigger', 'pinned'], queryFn: getPinnedNotes, enabled: char === '@' && !typed, staleTime: 60_000 })
  const helpers = useQuery({ queryKey: ['search', 'helpers'], queryFn: () => globalSessionsApi.list({}), enabled: char === '@', staleTime: 30_000 })
  const tree = useQuery({ queryKey: ['search', 'tree'], queryFn: () => getNoteTree('Notebooks'), enabled: char === '#', staleTime: 60_000 })

  if (!trigger) return []
  if (char === '#') {
    const folders = typed ? matchFolders(tree.data?.tree, settled, PER_GROUP * 2) : topLevelFolders(tree.data?.tree, PER_GROUP * 2)
    return folders.map((f) => ({ id: `folder:${f.path}`, kind: 'folders', title: f.trail, sub: f.path, reference: folderReference(f.path) }))
  }
  const notes = typed ? (found.data?.results ?? []).map((h) => ({ path: h.notePath, title: h.title })) : (pinned.data?.notes ?? []).map((n) => ({ path: n.path, title: n.title }))
  const taskRows = (typed ? matchTasks(tasks, settled) : tasks.filter((x) => !x.archived_at)).slice(0, PER_GROUP)
  const helperRows = (typed ? matchHelpers(helpers.data ?? [], settled) : []).slice(0, PER_GROUP)
  return [
    ...notes.slice(0, PER_GROUP).map((n): TriggerItem => ({ id: `note:${n.path}`, kind: 'notes', title: n.title, sub: n.path.split('/').slice(1, -1).join(' › '), reference: noteReference(n.path) })),
    ...taskRows.map((x): TriggerItem => ({ id: `task:${x.id}`, kind: 'tasks', title: x.title, sub: `${x.display_id} · ${STATUS_LABELS[x.status]}`, reference: taskReference(x.display_id) })),
    ...helperRows.map((s): TriggerItem => ({ id: `helper:${s.id}`, kind: 'helpers', title: sessionTitle(s), sub: `${agentName(s.harness)} · ${computerName(s.host)}`, reference: helperReference(s.id) })),
  ]
}

interface Props {
  items: TriggerItem[]
  active: number
  query: string
  onPick: (item: TriggerItem) => void
}

/** The short list above the box for "@" and "#". Arrow keys, Enter and Tab are heard by the box itself. */
export function TriggerMenu({ items, active, query, onPick }: Props) {
  let last: TriggerKind | null = null
  return (
    <div className="hq-trig" role="listbox" aria-label={t('chat_trigger_label')}>
      {items.length === 0 && <p className="a-body-sm a-muted hq-trig-none">{query ? t('chat_trigger_none') : t('chat_trigger_hint')}</p>}
      {items.map((item, i) => {
        const Icon = ICON[item.kind]
        const heading = item.kind !== last
        last = item.kind
        return (
          <div key={item.id}>
            {heading && <p className="a-grp hq-trig-grp">{t(GROUP_KEY[item.kind])}</p>}
            <div
              id={`hq-trig-${i}`}
              role="option"
              aria-selected={i === active}
              className="hq-trig-row"
              // The box keeps focus: a press picks without taking it.
              onMouseDown={(e) => {
                e.preventDefault()
                onPick(item)
              }}
            >
              <Icon className="a-ic xs" aria-hidden="true" />
              <span className="hq-trig-main">
                <b>{item.title}</b>
                {item.sub && <span className="a-cap a-muted">{item.sub}</span>}
              </span>
            </div>
          </div>
        )
      })}
      <p className="a-cap a-muted hq-trig-keys">{t('chat_trigger_keys')}</p>
    </div>
  )
}
