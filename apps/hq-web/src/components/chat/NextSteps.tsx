import { t } from '~/strings'
import type { NextStep } from '~/store/threadStore'

/** Two to four things to do next with what the reply made, in the slot above the box. A tap sends the request. */
export function NextSteps({ steps, onPick }: { steps: NextStep[]; onPick: (step: NextStep) => void }) {
  return (
    <div className="hq-nextsteps" role="group" aria-label={t('chat_next_steps')}>
      <span className="a-cap a-muted">{t('chat_next_steps')}</span>
      {steps.map((step) => (
        <button key={step.label} type="button" className="a-btn secondary compact" title={step.prompt} onClick={() => onPick(step)}>
          {step.label}
        </button>
      ))}
    </div>
  )
}
