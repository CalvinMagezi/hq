import { useEffect, useState } from 'react'
import { MessageSquareText, CalendarCheck, FilePlus } from 'lucide-react'
import { startSuggestions } from '~/lib/chatStart'
import { t } from '~/strings'
import { GetStartedCard } from '../vault/GetStartedCard'
import { HQ_MARK } from './ChatBubble'

const ICONS = [MessageSquareText, CalendarCheck, FilePlus]

/** A new chat: the phone's heading and line, three suggestions, and Get started until it is done. */
export function StartScreen({ onPick, noteAttached = false }: { onPick?: (text: string) => void; noteAttached?: boolean }) {
  // The card runs queries; rendered during the build's prerender, their cache timers keep the build from finishing.
  const [live, setLive] = useState(false)
  useEffect(() => setLive(true), [])
  return (
    <div className="flex flex-col items-center text-center py-10 sm:py-16 gap-3">
      <img src={HQ_MARK} alt="" className="w-12 h-12 object-contain opacity-80" />
      <h1 className="text-xl font-semibold">{t('chat_what_would_you_like')}</h1>
      <p className="text-sm max-w-md" style={{ color: 'var(--ink-muted)' }}>{t('chat_hq_can_read_your')}</p>
      <div className="mt-2 grid gap-2 w-full max-w-md">
        {startSuggestions(noteAttached).map((text, i) => {
          const Icon = ICONS[i]
          return (
            <button
              key={text}
              type="button"
              onClick={() => onPick?.(text)}
              className="min-h-11 px-4 py-2 rounded-xl border border-white/10 flex items-center gap-3 text-left text-sm hover:bg-white/5"
            >
              <Icon className="w-4 h-4 shrink-0" aria-hidden="true" />
              {text}
            </button>
          )
        })}
      </div>
      {live && (
        <div className="w-full max-w-md text-left mt-6">
          <GetStartedCard compact />
        </div>
      )}
    </div>
  )
}
