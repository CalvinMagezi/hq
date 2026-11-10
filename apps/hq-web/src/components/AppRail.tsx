import { t } from '~/strings'
import { PlaceLinks, SettingsRailLink } from './Places'

export { inboxNeedsYou } from './Places'

/** The navigation from 768 px: Notes, Chat and Inbox with their names, Chat raised, Settings at the foot. Phones use the BottomNav. */
export function AppRail() {
  return (
    <nav className="a-rail" aria-label={t('nav_places')}>
      <PlaceLinks kind="rail" />
      <span className="a-rail-gap" />
      <SettingsRailLink />
    </nav>
  )
}
