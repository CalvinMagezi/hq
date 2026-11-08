import { t } from '../../../hq-web/src/strings'

/** The names HQ gives the services it offers and forwards (crates/hq-core/src/config/bridge.rs). */
const GATE_SERVICE = 'gate'
const MODEL_SERVICE = 'llm'
const WEB_SERVICE = 'web'

export interface ServiceWords {
  title: string
  note: string
}

/** How a service this computer offers reads on the Devices tab; a name from config.yaml stays as written. */
export function offeredWords(name: string): ServiceWords {
  switch (name) {
    case GATE_SERVICE:
      return { title: t('dev_shared_helpers'), note: t('dev_shared_helpers_note') }
    case MODEL_SERVICE:
      return { title: t('dev_shared_model'), note: t('dev_shared_model_note') }
    case WEB_SERVICE:
      return { title: t('dev_shared_web_here'), note: t('dev_shared_web_here_note') }
    default:
      return { title: name, note: t('dev_shared_custom_note') }
  }
}

/** How a forward to a paired device's service reads; only the hub's web pages have their own words. */
export function forwardTitle(service: string): string {
  return service === WEB_SERVICE ? t('dev_shared_web') : service
}

/** The line under a forward: the HQ window uses the web one; another is one set in config.yaml. */
export function forwardNote(service: string): string {
  return service === WEB_SERVICE ? t('dev_shared_web_note') : t('dev_shared_custom_note')
}
