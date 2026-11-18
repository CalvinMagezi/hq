// How the work pane behaves in this browser: it opens by itself, only from a card, or never (a card opens the
// note as a page in the Vault). The choice is per browser, like "Enter sends".
export type PaneMode = 'auto' | 'card' | 'never'

export const PANE_MODES: PaneMode[] = ['auto', 'card', 'never']
const KEY = 'hq-pane-mode'

export function readPaneMode(store: Pick<Storage, 'getItem'> = localStorage): PaneMode {
  try {
    const v = store.getItem(KEY)
    return PANE_MODES.includes(v as PaneMode) ? (v as PaneMode) : 'auto'
  } catch {
    return 'auto'
  }
}

export function writePaneMode(mode: PaneMode, store: Pick<Storage, 'setItem'> = localStorage): void {
  try {
    store.setItem(KEY, mode)
  } catch {
    // Blocked storage: the choice lasts until the page is closed.
  }
}
