import { useEffect } from 'react'
import { fetchTaskCheckpointClient, type TaskItem } from '~/lib/tasksApi'
import { usePolled } from '../sessions/usePolled'
import { SectionLabel } from './taskFields'
import { formatLocalTime } from './timeFormat'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { t } from '~/strings'
import './leaf.css'

const CHECKPOINT_POLL_MS = 300_000

function Part({ label, text }: { label: string; text: string }) {
  if (!text) return null
  return (
    <div className="a-drow l-tall l-col">
      <div className="a-label">{label}</div>
      <p className="a-body-sm l-pre">{text}</p>
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
      <div className="a-card l-lines">
        <Part label={t('task_cp_so_far')} text={checkpoint.summary} />
        <Part label={t('task_cp_next')} text={checkpoint.next_step} />
        <Part label={t('task_cp_questions')} text={checkpoint.open_questions} />
        {checkpoint.files.length > 0 && (
          <div className="a-drow l-tall l-col">
            <div className="a-label">{t('task_cp_files')}</div>
            <ul className="a-cap l-break">
              {checkpoint.files.map((file) => (
                <li key={file}>{file}</li>
              ))}
            </ul>
          </div>
        )}
        {(checkpoint.suggested_skills?.length ?? 0) > 0 && (
          <div className="a-drow l-tall l-col">
            <div className="a-label">{t('task_cp_skills')}</div>
            <ul className="a-cap l-break">
              {checkpoint.suggested_skills?.map((name) => (
                <li key={name}>{name}</li>
              ))}
            </ul>
          </div>
        )}
        <p className="a-cap a-muted l-pad">
          {t('task_cp_left_by', { who: checkpoint.actor, when: formatLocalTime(checkpoint.created_at) })}
        </p>
      </div>
    </section>
  )
}
