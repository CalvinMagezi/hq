import { NextResponse } from 'next/server'
import { HetznerError } from './hetzner'
import { isHetznerToken } from './validate'
import { pinnedFromEnv, type Pinned } from './userData'

export class BadRequest extends Error {}

const NO_STORE = { 'Cache-Control': 'no-store' }

export function fail(message: string, status: number) {
  return NextResponse.json({ error: message }, { status, headers: NO_STORE })
}

/**
 * Wraps a wizard route. The Hetzner token is read from the JSON body, handed to `run`, and never
 * logged, stored or echoed: every error below is built from fixed or Hetzner-supplied text.
 */
export async function route<B extends { token: string }>(
  req: Request,
  run: (body: B, pinned: Pinned) => Promise<unknown>,
) {
  let body: B
  try {
    body = (await req.json()) as B
  } catch {
    return fail('The request was not valid JSON.', 400)
  }
  if (!isHetznerToken(body?.token)) return fail('Enter a Hetzner Cloud API token.', 400)
  const pinned = pinnedFromEnv()
  if (!pinned) return fail('This wizard is not configured with a bootstrap to deploy.', 503)
  try {
    return NextResponse.json(await run(body, pinned), { headers: NO_STORE })
  } catch (e) {
    if (e instanceof BadRequest) return fail(e.message, 400)
    if (e instanceof HetznerError) return fail(e.message, e.status === 401 || e.status === 403 || e.status === 404 || e.status === 429 ? e.status : 502)
    return fail('Something went wrong on our side.', 500)
  }
}
