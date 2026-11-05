/** A session event as the registry records it; `detail` is JSON text for an `answered` event. */
export interface SessionEvent {
  kind: string
  by?: string | null
  detail?: string | null
  at?: string | null
}

export interface AutoAnswer {
  at: string | null
  by: string
  kind: string
  declined: boolean
}

const OWNER = 'owner'

/** The answers someone other than the owner gave a helper's dialogs, oldest first, read from its `answered` events. */
export function autoAnswers(events: SessionEvent[] | undefined): AutoAnswer[] {
  const out: AutoAnswer[] = []
  for (const e of events ?? []) {
    if (e.kind !== 'answered') continue
    let detail: { by?: string; choice?: unknown; kind?: string } = {}
    try {
      detail = JSON.parse(e.detail ?? '{}') as typeof detail
    } catch {
      continue
    }
    const by = detail.by ?? e.by ?? ''
    if (!by || by === OWNER) continue
    out.push({ at: e.at ?? null, by, kind: detail.kind ?? 'dialog', declined: detail.choice === 'decline' })
  }
  return out.sort((a, b) => (a.at ?? '').localeCompare(b.at ?? ''))
}
