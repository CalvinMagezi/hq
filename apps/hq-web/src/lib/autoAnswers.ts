/** One answer as `GET /api/harness-sessions/{id}/answers` lists it: HQ's own, never the owner's. */
export interface ServerAnswer {
  by?: string | null
  at?: string | null
  kind?: string | null
  choice?: unknown
  registered?: unknown
}

export interface AutoAnswer {
  at: string | null
  by: string
  kind: string
  declined: boolean
}

/** The answers HQ gave a helper's dialogs, oldest first, from the server's answers list. */
export function autoAnswers(answers: ServerAnswer[] | undefined): AutoAnswer[] {
  const out: AutoAnswer[] = []
  for (const a of answers ?? []) {
    if (!a.by || a.by === 'owner') continue
    out.push({ at: a.at ?? null, by: a.by, kind: a.kind ?? 'dialog', declined: a.choice === 'decline' })
  }
  return out.sort((x, y) => (x.at ?? '').localeCompare(y.at ?? ''))
}
