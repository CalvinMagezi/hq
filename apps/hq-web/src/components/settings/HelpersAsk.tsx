import { t, type StringKey } from '~/strings'
import { ASK_LEVELS, usePrefs, type AskLevel } from '~/lib/prefsApi'

const WORDS: Record<AskLevel, { title: StringKey; about: StringKey }> = {
  nothing: { title: 'settings_helper_policy_nothing', about: 'settings_helper_policy_nothing_about' },
  loses_work: { title: 'settings_helper_policy_loses', about: 'settings_helper_policy_loses_about' },
  everything: { title: 'settings_helper_policy_everything', about: 'settings_helper_policy_everything_about' },
}

/**
 * Which helper requests reach the owner: nothing (their agents handle it), only what would lose work, or
 * everything. It is the phone's setting, kept by HQ, so the two are one rule. Changes apply to new requests.
 */
export function HelpersAsk() {
  const { prefs, failed, set } = usePrefs()
  const chosen = prefs?.['helpers.ask_level']
  return (
    <section aria-labelledby="st-ask-title" className="st-ask">
      <h3 id="st-ask-title" className="a-grp">
        {t('settings_helper_policy_title')}
      </h3>
      <p className="a-muted a-body-sm">{t('settings_helper_policy_intro')}</p>
      <div className="a-set" role="radiogroup" aria-labelledby="st-ask-title">
        {ASK_LEVELS.map((level) => (
          <button key={level} type="button" role="radio" aria-checked={chosen === level} disabled={!prefs} className="a-li" onClick={() => set({ key: 'helpers.ask_level', value: level })}>
            <div className="a-li-main">
              <b>{t(WORDS[level].title)}</b>
              <span>{t(WORDS[level].about)}</span>
            </div>
            <span className={`st-radio${chosen === level ? ' is-on' : ''}`} aria-hidden="true" />
          </button>
        ))}
      </div>
      {!prefs && !failed && <p className="a-body-sm a-muted">{t('settings_helper_policy_unknown')}</p>}
      {failed && (
        <p role="alert" className="a-body-sm a-warn-t">
          {t('prefs_failed')}
        </p>
      )}
    </section>
  )
}
