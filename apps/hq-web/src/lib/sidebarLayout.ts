/** Width limits and persistence for the Workbench sidebar, which the person can resize and collapse. */
export const SIDEBAR_MIN = 280
export const SIDEBAR_MAX = 560
export const SIDEBAR_DEFAULT = 380
/** Each arrow key press moves the resize handle by this many pixels. */
export const SIDEBAR_KEY_STEP = 16

export interface SidebarLayout {
  width: number
  collapsed: boolean
}

export const DEFAULT_SIDEBAR: SidebarLayout = { width: SIDEBAR_DEFAULT, collapsed: false }

export function clampSidebarWidth(width: number): number {
  if (!Number.isFinite(width)) return SIDEBAR_DEFAULT
  return Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, Math.round(width)))
}

/** Parses the stored value, falling back to the default for anything unreadable. */
export function parseSidebarLayout(raw: string | null): SidebarLayout {
  if (!raw) return DEFAULT_SIDEBAR
  try {
    const value = JSON.parse(raw) as Partial<SidebarLayout> | null
    if (!value || typeof value !== 'object') return DEFAULT_SIDEBAR
    return {
      width: typeof value.width === 'number' ? clampSidebarWidth(value.width) : SIDEBAR_DEFAULT,
      collapsed: value.collapsed === true,
    }
  } catch {
    return DEFAULT_SIDEBAR
  }
}
