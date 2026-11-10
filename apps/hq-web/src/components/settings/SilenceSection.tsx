import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { SILENCE_TOPICS, fetchSilences, silenceTopic, unsilenceTopic, type SilenceRow, type SilenceTopic, type Silences } from '~/lib/notificationsApi'
import { t, type StringKey } from '~/strings'

const KEY = ['inbox-silences']
const HOUR = 60
const NIGHT = 8 * 60

const TOPIC_NAME: Record<SilenceTopic, StringKey> = {
  helpers: 'topic_helpers',
  tasks: 'topic_tasks',
  tips: 'topic_tips',
  upkeep: 'topic_upkeep',
  email: 'topic_email',
}

const TOPIC_ABOUT: Record<SilenceTopic, StringKey> = {
  helpers: 'topic_helpers_about',
  tasks: 'topic_tasks_about',
  tips: 'topic_tips_about',
  upkeep: 'topic_upkeep_about',
  email: 'topic_email_about',
}

const untilWords = (until: string) => new Intl.DateTimeFormat(undefined, { weekday: 'short', hour: 'numeric', minute: '2-digit' }).format(new Date(until))

/** Where the pause stands, in one line. */
export function pauseLine(pause: SilenceRow): string | null {
  if (!pause.silenced) return null
  return pause.until ? t('silence_paused_until', { time: untilWords(pause.until) }) : t('silence_paused')
}

interface ViewProps {
  silences: Silences
  busy: boolean
  onPause: (minutes: number) => void
  onResume: () => void
  /** Flips one topic: silenced until turned back on, or on again. */
  onTopic: (topic: SilenceTopic, silence: boolean) => void
}

/** The pause and the topics, each a switch that reads "notifications on". */
export function SilenceView({ silences, busy, onPause, onResume, onTopic }: ViewProps) {
  const paused = pauseLine(silences.pause)
  const rowOf = (topic: SilenceTopic) => silences.topics.find((r) => r.topic === topic)
  return (
    <section className="flex flex-col gap-4" aria-labelledby="silence-title">
      <h2 id="silence-title" className="a-label">
        {t('silence_title')}
      </h2>
      <p className="a-muted a-body-sm">{t('silence_intro')}</p>
      <div className="a-set" style={{ margin: 0 }}>
        <div className="a-li">
          <div className="a-li-main">
            <b>{t('silence_pause')}</b>
            {paused && <span>{paused}</span>}
          </div>
          {paused ? (
            <button type="button" className="a-btn secondary compact" disabled={busy} onClick={onResume}>
              {t('silence_resume')}
            </button>
          ) : (
            <span className="a-btns">
              <button type="button" className="a-btn secondary compact" disabled={busy} onClick={() => onPause(HOUR)}>
                {t('silence_pause_hour')}
              </button>
              <button type="button" className="a-btn secondary compact" disabled={busy} onClick={() => onPause(NIGHT)}>
                {t('silence_pause_night')}
              </button>
            </span>
          )}
        </div>
        {SILENCE_TOPICS.map((topic) => {
          const row = rowOf(topic)
          const silenced = row?.silenced === true
          const state = !silenced ? t('silence_on') : row?.until ? t('silence_off_until', { time: untilWords(row.until) }) : t('silence_off')
          return (
            <div className="a-li" key={topic}>
              <div className="a-li-main">
                <b>{t(TOPIC_NAME[topic])}</b>
                <span style={{ whiteSpace: 'normal' }}>
                  {[state, t(TOPIC_ABOUT[topic])].join(' \u00b7 ')}
                </span>
              </div>
              <button type="button" role="switch" aria-checked={!silenced} aria-label={t(TOPIC_NAME[topic])} disabled={busy} onClick={() => onTopic(topic, !silenced)} className="a-sw" />
            </div>
          )
        })}
      </div>
    </section>
  )
}

/** Which topics may interrupt the owner. Kept on HQ, so it holds for every browser and the phone. */
export function SilenceSection() {
  const client = useQueryClient()
  const silences = useQuery({ queryKey: KEY, queryFn: fetchSilences }).data
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState(false)
  if (!silences) return null
  const run = async (change: typeof fetchSilences) => {
    setBusy(true)
    setFailed(false)
    try {
      client.setQueryData(KEY, await change())
    } catch {
      setFailed(true)
    } finally {
      setBusy(false)
    }
  }
  return (
    <>
      <SilenceView
        silences={silences}
        busy={busy}
        onPause={(minutes) => void run(() => silenceTopic('all', minutes))}
        onResume={() => void run(() => unsilenceTopic('all'))}
        onTopic={(topic, silence) => void run(() => (silence ? silenceTopic(topic) : unsilenceTopic(topic)))}
      />
      {failed && (
        <p className="a-muted a-body-sm" role="alert">
          {t('inbox_fail_no_reply')}
        </p>
      )}
    </>
  )
}
