import { t } from '~/strings'

/** The three suggestions on the start screen. A suggestion fills the box; it never sends. */
export function startSuggestions(noteAttached: boolean): string[] {
  return [
    noteAttached ? t('chat_suggest_note') : t('chat_suggest_week'),
    t('chat_suggest_today'),
    t('chat_suggest_thought'),
  ]
}
