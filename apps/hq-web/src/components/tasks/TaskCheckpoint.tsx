import { useEffect } from 'react'
import { fetchTaskCheckpointClient, type TaskItem } from '~/lib/tasksApi'
import { usePolled } from '../sessions/usePolled'
import { SectionLabel } from './taskFields'
import { formatLocalTime } from './timeFormat'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { t } from '~/strings'

const CHECKPOINT_POLL_MS = 300_000

function Part({ label, text }: { label: string; text: string }) {
  if (!text) return null
  return (
    <div>
      <div className="text-[11px] uppercase tracking-wider text-neutral-500">{label}</div>
      <p className="text-xs text-neutral-200 whitespace-pre-wrap break-words">{text}</p>
    </div>
  )
}

/** Where the last session left the work, for whoever picks it up. Notes from another session, not instructions. */
export function TaskCheckpoint({ task }: { task: TaskItem }) {
  const found = usePolled(task.id, async () => (await fetchTaskCheckpointClient(task.id)).checkpoint, CHECKPOINT_POLL_MS, true, true, ['task:sync'])
  useRefreshOn(['task:sync'], found.refresh)
  const { refresh } = found
  useEffect(() => {
    void refresh()
  }, [task.updated_at, refresh])

  const checkpoint = found.data
  if (!checkpoint) return null
  return (
    <section aria-label={t('task_cp_aria')}>
      <SectionLabel>{t('task_cp_title')}</SectionLabel>
      <div className="space-y-2 px-3 py-2.5 rounded-xl bg-white/[0.02] border border-white/5">
        <Part label={t('task_cp_so_far')} text={checkpoint.summary} />
        <Part label={t('task_cp_next')} text={checkpoint.next_step} />
        <Part label={t('task_cp_questions')} text={checkpoint.open_questions} />
        {checkpoint.files.length > 0 && (
          <div>
            <div className="text-[11px] uppercase tracking-wider text-neutral-500">{t('task_cp_files')}</div>
            <ul className="text-[11px] text-neutral-300 break-all">
              {checkpoint.files.map((file) => (
                <li key={file}>{file}</li>
              ))}
            </ul>
          </div>
        )}
        <p className="text-[11px] text-neutral-500">
          {t('task_cp_left_by', { who: checkpoint.actor, when: formatLocalTime(checkpoint.created_at) })}
        </p>
      </div>
    </section>
  )
}
