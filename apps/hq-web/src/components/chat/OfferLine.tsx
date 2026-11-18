import { useState } from 'react'
import { t } from '~/strings'
import type { ChatOffer } from '~/store/threadStore'
import { canSchedule, scheduleChoices } from '~/lib/offersApi'

/** One thing HQ can also do here, with Yes, Later and Not now. Not a notification: it sits in the slot above the box. */
export function OfferLine({ offer, onAnswer, onSchedule }: { offer: ChatOffer; onAnswer: (yes: boolean) => void; onSchedule?: (at: number) => void }) {
  const [picking, setPicking] = useState(false)
  if (picking && onSchedule) {
    return (
      <div className="hq-offer">
        <span className="a-body-sm a-muted">{offer.line}</span>
        <span className="a-btns">
          {scheduleChoices(new Date()).map((c) => (
            <button key={c.key} type="button" className="a-btn secondary compact" onClick={() => onSchedule(c.at)}>
              {t(c.key)}
            </button>
          ))}
          <button type="button" className="a-btn quiet compact" onClick={() => setPicking(false)}>
            {t('chat_offer_back')}
          </button>
        </span>
      </div>
    )
  }
  return (
    <div className="hq-offer">
      <span className="a-body-sm a-muted">{offer.line}</span>
      <span className="a-btns">
        <button type="button" className="a-btn secondary compact" onClick={() => onAnswer(true)}>
          {t('chat_offer_yes')}
        </button>
        {onSchedule && canSchedule(offer) && (
          <button type="button" className="a-btn secondary compact" onClick={() => setPicking(true)}>
            {t('chat_offer_schedule')}
          </button>
        )}
        <button type="button" className="a-btn quiet compact" onClick={() => onAnswer(false)}>
          {t('chat_offer_not_now')}
        </button>
      </span>
    </div>
  )
}
