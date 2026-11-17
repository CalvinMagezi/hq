import { useState } from 'react'
import { t, type StringKey } from '~/strings'
import { readEnterSends, writeEnterSends } from '~/lib/enterRule'
import { PANE_MODES, readPaneMode, writePaneMode, type PaneMode } from '~/lib/paneMode'
import { usePrefs } from '~/lib/prefsApi'

const PANE_TEXT: Record<PaneMode, { title: StringKey; line: StringKey }> = {
  auto: { title: 'chat_set_pane_auto_title', line: 'chat_set_pane_auto_line' },
  card: { title: 'chat_set_pane_card_title', line: 'chat_set_pane_card_line' },
  never: { title: 'chat_set_pane_never_title', line: 'chat_set_pane_never_line' },
}

const COARSE = '(pointer: coarse)'

/** Every key the app answers, by place; Ctrl / opens this page. Only keys that really work are listed. */
const KEYS: { place: StringKey; rows: { what: StringKey; keys: string }[] }[] = [
  {
    place: 'chat_keys_anywhere',
    rows: [
      { what: 'key_palette', keys: 'Ctrl K' },
      { what: 'key_new_note', keys: 'Ctrl Alt N' },
      { what: 'key_new_chat', keys: 'Ctrl Shift O' },
      { what: 'key_pane', keys: 'Ctrl .' },
      { what: 'key_keys', keys: 'Ctrl /' },
    ],
  },
  {
    place: 'chat_keys_chat',
    rows: [
      { what: 'key_send', keys: 'Enter' },
      { what: 'key_newline', keys: 'Shift Enter' },
      { what: 'key_stop', keys: 'Esc' },
      { what: 'key_edit_last', keys: 'Up' },
      { what: 'key_copy_last', keys: 'Ctrl Shift C' },
      { what: 'key_chat_move', keys: 'Alt Up, Alt Down' },
    ],
  },
  {
    place: 'chat_keys_inbox',
    rows: [
      { what: 'key_inbox_move', keys: 'J, K' },
      { what: 'key_inbox_back', keys: 'Esc' },
      { what: 'key_inbox_clear', keys: 'E' },
      { what: 'key_inbox_later', keys: 'H' },
      { what: 'key_inbox_undo', keys: 'Z' },
    ],
  },
  {
    place: 'chat_keys_vault',
    rows: [
      { what: 'key_vault_back', keys: 'Alt Left, Alt Right' },
      { what: 'key_vault_up', keys: 'Alt Up' },
      { what: 'key_vault_tree', keys: 'Ctrl B' },
    ],
  },
]

/** How Chat behaves in this browser: whether Enter sends, what the work pane does, and the keys. */
export function ChatSettings() {
  const [enter, setEnter] = useState<boolean>(() => readEnterSends() ?? !window.matchMedia(COARSE).matches)
  const [pane, setPane] = useState<PaneMode>(() => readPaneMode())
  const { prefs, failed, set } = usePrefs()
  const titles = prefs?.['chat.auto_title']
  return (
    <div className="st-chat">
      <p className="a-muted a-body-sm">{t('chat_set_saved')}</p>

      <h3 className="a-grp">{t('chat_set_sending')}</h3>
      <div className="a-set">
        <div className="a-li">
          <div className="a-li-main">
            <b id="st-enter-label">{t('chat_set_enter_title')}</b>
            <span>{t('chat_set_enter_line')}</span>
          </div>
          <button
            type="button"
            role="switch"
            className="a-sw"
            aria-checked={enter}
            aria-labelledby="st-enter-label"
            onClick={() => {
              writeEnterSends(!enter)
              setEnter(!enter)
            }}
          />
        </div>
      </div>

      <h3 className="a-grp">{t('chat_set_pane_title')}</h3>
      <p className="a-muted a-body-sm">{t('chat_set_pane_line')}</p>
      <div className="a-set" role="radiogroup" aria-label={t('chat_set_pane_title')}>
        {PANE_MODES.map((mode) => (
          <button
            key={mode}
            type="button"
            role="radio"
            aria-checked={pane === mode}
            className="a-li"
            onClick={() => {
              writePaneMode(mode)
              setPane(mode)
            }}
          >
            <div className="a-li-main">
              <b>{t(PANE_TEXT[mode].title)}</b>
              <span>{t(PANE_TEXT[mode].line)}</span>
            </div>
            <span className={`st-radio${pane === mode ? ' is-on' : ''}`} aria-hidden="true" />
          </button>
        ))}
      </div>

      <h3 className="a-grp">{t('chat_set_titles_title')}</h3>
      <div className="a-set">
        <div className="a-li">
          <div className="a-li-main">
            <b id="st-titles-label">{t('chat_set_titles_item')}</b>
            <span>{t('chat_set_titles_line')}</span>
          </div>
          <button
            type="button"
            role="switch"
            className="a-sw"
            aria-checked={titles === true}
            aria-labelledby="st-titles-label"
            disabled={titles === undefined}
            onClick={() => set({ key: 'chat.auto_title', value: !titles })}
          />
        </div>
      </div>
      <p className="a-muted a-body-sm">{t('chat_set_titles_shared')}</p>
      {failed && (
        <p role="alert" className="a-body-sm a-warn-t">
          {t('prefs_failed')}
        </p>
      )}

      <h3 className="a-grp">{t('chat_set_keys_title')}</h3>
      <p className="a-muted a-body-sm">{t('chat_set_keys_line')}</p>
      {KEYS.map((group) => (
        <section key={group.place} aria-label={t(group.place)}>
          <p className="a-cap a-muted st-keys-place">{t(group.place)}</p>
          <div className="a-set">
            {group.rows.map((row) => (
              <div key={row.what} className="a-drow">
                <span>{t(row.what)}</span>
                <span className="st-keys">{row.keys}</span>
              </div>
            ))}
          </div>
        </section>
      ))}
    </div>
  )
}
