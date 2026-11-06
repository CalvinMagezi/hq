import { useEffect, useState } from 'react'
import { MessageSquareText, CalendarCheck, FilePlus } from 'lucide-react'
import { startSuggestions } from '~/lib/chatStart'
import { t } from '~/strings'
import { GetStartedCard } from '../vault/GetStartedCard'

const ICONS = [MessageSquareText, CalendarCheck, FilePlus]

/** A new chat: the phone's heading and line, three suggestions, and Get started until it is done. */
export function StartScreen({ onPick, noteAttached = false }: { onPick?: (text: string) => void; noteAttached?: boolean }) {
  // The card runs queries; rendered during the build's prerender, their cache timers keep the build from finishing.
  const [live, setLive] = useState(false)
  useEffect(() => setLive(true), [])
  return (
    <div className="hq-start">
      <div className="hq-start-head">
        <span className="hq-start-mark">
          <span className="a-mark" />
        </span>
        <h1 className="a-h2">{t('chat_what_would_you_like')}</h1>
        <p className="a-body a-muted">{t('chat_hq_can_read_your')}</p>
      </div>
      <div className="a-btns hq-start-picks">
        {startSuggestions(noteAttached).map((text, i) => {
          const Icon = ICONS[i]
          return (
            <button key={text} type="button" className="a-btn secondary compact" onClick={() => onPick?.(text)}>
              <Icon className="a-ic sm" aria-hidden="true" />
              {text}
            </button>
          )
        })}
      </div>
      {live && <GetStartedCard compact />}
    </div>
  )
}
