/** What the viewers use to change the note: the one function that applies an edit, and whether editing is on. */
export interface Editor {
  on: boolean
  apply: (change: (current: string) => string | null) => boolean
  /** Where the note lives, for the structure calls that go to HQ directly. */
  path?: string
  /** The hash this page last saw, for the same calls. */
  hash: () => string | undefined
  /** An edit is waiting for HQ to be reachable: calls that go to HQ directly are held back. */
  waiting: boolean
  /** Load the note again (after HQ changed it). */
  reload: () => void
}
