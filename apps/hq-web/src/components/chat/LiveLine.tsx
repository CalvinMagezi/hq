import { useEffect, useState } from 'react'
import { Square } from 'lucide-react'
import { t } from '~/strings'
import type { ToolStep } from '~/store/threadStore'
import { idleLine, liveSentence, liveText, secondsRunning, stopOffered } from '~/lib/liveLine'

/** A clock that ticks once a second while the component is on screen. */
function useNow(everyMs: number): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), everyMs)
    return () => window.clearInterval(id)
  }, [everyMs])
  return now
}

interface Props {
  steps: ToolStep[]
  startedAt?: number
  /** Words of the answer have started to arrive. */
  writing: boolean
  /** The model's reasoning is arriving but no answer words yet. */
  thinking?: boolean
  onStop?: () => void
}

/**
 * The one line above a streaming reply: the step running now as a sentence, replaced by the next step
 * and never stacked. The answer's own words arrive below it. After ten seconds Stop sits on the line.
 */
export function LiveLine({ steps, startedAt, writing, thinking = false, onStop }: Props) {
  const now = useNow(1000)
  const sentence = liveSentence(steps)
  const idle = idleLine(steps, thinking)
  const text = liveText(sentence) ?? (writing ? t('chat_live_writing') : t(idle.key, idle.params))
  const seconds = secondsRunning(startedAt, now)
  return (
    <p className="hq-live" role="status">
      <span className="a-spin" aria-hidden="true" />
      <span className="hq-live-text">{text}</span>
      {0 < seconds && <span className="a-cap a-muted hq-live-time">{t('chat_live_seconds', { n: String(seconds) })}</span>}
      {onStop && stopOffered(startedAt, now) && (
        <button type="button" className="a-btn quiet compact hq-live-stop" onClick={onStop}>
          <Square className="a-ic xs" aria-hidden="true" />
          {t('chat_stop')}
        </button>
      )}
    </p>
  )
}
