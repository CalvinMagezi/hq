import { hqJson } from './hqAuth'

/** What HQ can do on this install, in the person's words (`GET /api/capabilities`). */
export interface Capability {
  id: string
  group: string
  says: string
  /** Something to type to try it; a chip puts it in the composer. */
  example: string
  state: 'ready' | 'one_step'
  /** Present when `state` is `one_step`: what would make it work. */
  step?: string
  used: boolean
}

export interface CapabilitiesView {
  suggestions: Capability[]
  capabilities: Capability[]
  needs_model: boolean
}

export const fetchCapabilities = () => hqJson<CapabilitiesView>('/api/capabilities')

/** The empty chat shows these. With no model key there are none: setup comes first. */
export function emptyChatChips(view: CapabilitiesView | null | undefined, max = 4): Capability[] {
  if (!view || view.needs_model) return []
  return view.suggestions.filter((c) => c.state === 'ready' && c.example.trim()).slice(0, max)
}
