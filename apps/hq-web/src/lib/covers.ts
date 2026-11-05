// Places drawn without the shell: one main action each, returning to the start place (web wave 4, 02).
const COVERS = ['/setup', '/connect', '/signin']

export function isCover(pathname: string): boolean {
  return COVERS.some((c) => pathname === c || pathname.startsWith(`${c}/`))
}

/** A browser with no stored sign-in for this HQ opens the full page; one that was signed out keeps the shell. */
export function needsSignInPage(s: { signInNeeded: boolean; hadSignIn: boolean }): boolean {
  return s.signInNeeded && !s.hadSignIn
}

/** Covers that explain a refused sign-in themselves, so the sign-in redirect and the empty view leave them alone. */
export function ownsSignedOut(pathname: string): boolean {
  return pathname === '/signin' || pathname === '/connect'
}
