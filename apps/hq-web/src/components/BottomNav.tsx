import { t } from '~/strings'
import { PlaceLinks } from './Places'

/** The phone's bottom bar below 768 px: the same three places (Notes, Chat, Inbox), Chat raised on the HQ mark. */
export function BottomNav() {
  return (
    <nav className="a-nav" aria-label={t('nav_places')}>
      <PlaceLinks kind="nav" />
    </nav>
  )
}
