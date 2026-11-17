// The @ and # triggers in the message box: a word that starts with one opens a short list, and picking
// from it puts a reference in the text. A reference is a backticked path or name, the form Ask HQ already
// uses, so HQ gets the real thing and not a guess at its name. One Backspace removes the whole reference.
import type { NoteTreeNode } from './vaultApi'

export type TriggerChar = '@' | '#'

export interface Trigger {
  char: TriggerChar
  /** What was typed after the character. */
  query: string
  /** Where the character sits in the text. */
  start: number
}

/** The trigger the caret is in: a word beginning with @ or #, with nothing after it but its own letters. */
export function triggerAt(text: string, caret: number): Trigger | null {
  const before = text.slice(0, caret)
  const match = /(^|\s)([@#])([^\s@#`]*)$/.exec(before)
  if (!match) return null
  const char = match[2] as TriggerChar
  return { char, query: match[3], start: before.length - match[3].length - 1 }
}

/** The text with the trigger word replaced by the reference and a space after it, and where the caret goes. */
export function insertReference(text: string, trigger: Trigger, caret: number, reference: string): { text: string; caret: number } {
  const rest = text.slice(caret)
  // A space after the reference, unless one already follows.
  const gap = /^\s/.test(rest) ? '' : ' '
  return { text: `${text.slice(0, trigger.start)}${reference}${gap}${rest}`, caret: trigger.start + reference.length + 1 }
}

export const noteReference = (path: string) => `\`${path}\``
export const folderReference = (path: string) => `\`${path}/\``
export const taskReference = (displayId: string) => `\`task ${displayId}\``
export const helperReference = (id: string) => `\`helper ${id}\``

// What a reference this box wrote starts with; any other backticked word is the person's own code.
const REFERENCE = /^(Notebooks\/|task |helper )/

/** The span of a reference that ends exactly at the caret, so one Backspace can take it all. */
export function referenceBefore(text: string, caret: number): [number, number] | null {
  if (caret < 3 || text[caret - 1] !== '`') return null
  const open = text.lastIndexOf('`', caret - 2)
  if (open < 0 || text.slice(open, caret).includes('\n')) return null
  return REFERENCE.test(text.slice(open + 1, caret - 1)) ? [open, caret] : null
}

/** The folders directly under the Vault's top folder, for "#" with nothing typed after it. */
export function topLevelFolders(root: NoteTreeNode | undefined, limit = 8): { path: string; trail: string }[] {
  return (root?.children ?? [])
    .filter((c) => c.type === 'dir' && !c.name.startsWith('_'))
    .slice(0, limit)
    .map((c) => ({ path: c.path, trail: c.name }))
}
