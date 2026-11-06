import { t } from '~/strings'
import { PlaceLinks } from './Places'

/** The phone's bottom bar below 768 px: the same five places, Chat raised on the HQ mark. */
export function BottomNav() {
  return (
    <nav className="a-nav" aria-label={t('nav_places')}>
      <PlaceLinks kind="nav" />
    </nav>
  )
}
