import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

// web-matches-phone wave A: the shell is the design's `.a-app > .a-shell`, as the boards draw it.
const root = join(import.meta.dir, '..', '..')
const read = (p: string) => readFileSync(join(root, p), 'utf8')

describe('the shell (AlignedShell)', () => {
  const shell = read('src/routes/__root.tsx')
  test('the window is the design root, with a skip link, the top bar and the main area', () => {
    for (const part of ['a-app hq-root', '"a-shell"', '"a-skip"', '"a-top"', '"a-top-actions"', '"a-main"', 'a-ib a-top-gear', '"a-find"']) {
      expect(shell, part).toContain(part)
    }
  })
  test('the older shell is gone: no glass, no "HQ Vault", no overlay chat', () => {
    for (const gone of ['hq-glass-pane', 'hq-ambient-bg', 'HQ Vault', 'GlobalChatOverlay', 'hq-header']) expect(shell, gone).not.toContain(gone)
  })
  test('the places carry their names, Chat is raised on the HQ mark, Settings ends the rail', () => {
    const places = read('src/components/Places.tsx')
    for (const part of ['a-rail-chat', 'a-nav-chat', '"a-hqb"', '"a-mark"', '"a-pill"', '"a-badge"']) expect(places, part).toContain(part)
    expect(read('src/components/AppRail.tsx')).toMatch(/a-rail[\s\S]*a-rail-gap[\s\S]*SettingsRailLink/)
    expect(read('src/components/BottomNav.tsx')).toContain('"a-nav"')
  })
})

describe('foundations (AlignedFoundations)', () => {
  const aligned = read('src/styles/aligned-ui.css')
  const app = read('app.css')
  test('the stylesheet has the rules the canvas added after the first copy', () => {
    for (const rule of [
      '.a-mark{', 'ClearSans-BoldItalic.woff2', '.a-field.is-error .a-input{border-color:var(--warn)}',
      '@container app (max-width:359px){.a-top-more{display:inline-flex}', '.a-panes.no-list .a-pane-detail{display:flex}',
      '.a-sw[aria-checked="true"]{background:var(--ink)', '.a-banner.work>.a-ic{color:var(--brand-blue)}',
    ]) expect(aligned, rule).toContain(rule)
  })
  test('Clear Sans, no glass: no other web fonts and no blur', () => {
    expect(app).not.toContain('@fontsource')
    expect(app).not.toMatch(/backdrop-filter/)
    expect(app).toMatch(/--font-body:\s*var\(--font-sans\)/)
  })
})
