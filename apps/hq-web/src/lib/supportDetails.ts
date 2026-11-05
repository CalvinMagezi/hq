// The text "Copy details" puts on the clipboard (docs/plans/ux-run/web/wave-4/08-settings.md S2):
// versions only, with anything that looks like a key blanked before the owner reads it.

const KEY_SHAPES = [/\b(sk|pk|ghp|gho|xox[bap])[-_][A-Za-z0-9_-]{8,}/g, /\bBearer\s+[A-Za-z0-9._~+/=-]{8,}/gi, /\b[A-Za-z0-9_-]{32,}\b/g]

export function blankKeys(text: string): string {
  return KEY_SHAPES.reduce((out, re) => out.replace(re, '[blanked]'), text)
}

export function supportDetails(lines: string[]): string {
  return blankKeys(lines.filter(Boolean).join('\n'))
}
