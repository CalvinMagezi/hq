#!/usr/bin/env node
// Loads the suite views (sheet, slides, dashboard, doc) from a static fixture at 360x800 with touch, and checks:
//   - no horizontal scroll on the page,
//   - every control a finger can press is at least 44px by 44px,
//   - nothing throws in the page.
// By hand, from apps/hq-web (not part of CI):
//   bun install && node scripts/suite-mobile-check.cjs [--shots <dir>] [--theme light] [--density compact]
// It bundles scripts/suite-fixture/entry.tsx with `bun build`, serves it from a temporary folder on loopback
// and drives the playwright and chromium that are already installed (no `playwright install`).
const { chromium } = require('playwright')
const { execFileSync } = require('node:child_process')
const { mkdtempSync, writeFileSync, readFileSync, mkdirSync, readdirSync } = require('node:fs')
const { createServer } = require('node:http')
const { tmpdir } = require('node:os')
const { join, extname } = require('node:path')

const MIN = 44
const arg = (name, fallback) => {
  const i = process.argv.indexOf(`--${name}`)
  return i >= 0 ? process.argv[i + 1] : fallback
}
const shots = arg('shots', null)
const theme = arg('theme', 'dark')
const density = arg('density', 'comfortable')

const root = join(__dirname, '..')
const out = mkdtempSync(join(tmpdir(), 'suite-fixture-'))
execFileSync('bun', ['build', join(root, 'scripts/suite-fixture/entry.tsx'), '--outdir', out, '--target', 'browser', '--external', '/fonts/*', '--external', '/icons/*', '--define', 'process.env.NODE_ENV="production"'], { cwd: root, stdio: 'inherit' })
writeFileSync(
  join(out, 'index.html'),
  `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>suite fixture</title>${readdirSync(out)
    .filter((f) => f.endsWith('.css'))
    .map((f) => `<link rel="stylesheet" href="/${f}">`)
    .join('')}<style>dialog{color:var(--ink)}/* no Tailwind here: the two utilities the markdown viewer metadata toggle uses */.w-full{width:100%}.min-h-\\[44px\\]{min-height:44px}</style></head><body style="margin:0;background:var(--surface-000,#000)"><div id="root"></div><script type="module" src="/entry.js"></script></body></html>`,
)
const TYPES = { '.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html' }
const server = createServer((req, res) => {
  const name = (req.url || '/').split('?')[0]
  const file = name === '/' ? 'index.html' : name.slice(1)
  // The fonts and icons the stylesheet names come from the app's public folder.
  for (const dir of [out, join(root, 'public')]) {
    try {
      const body = readFileSync(join(dir, file))
      res.writeHead(200, { 'content-type': TYPES[extname(file)] || (file.endsWith('.woff2') ? 'font/woff2' : 'application/octet-stream') })
      return res.end(body)
    } catch {
      // Try the next folder.
    }
  }
  res.writeHead(404).end()
})

/** Every thing a finger can press that is on screen, and its size. The row of a checkbox counts for the box. */
const measure = () => {
  const sel = 'button, a[href], select, textarea, input, [role="tab"], [role="gridcell"], summary, [tabindex="0"]'
  const bad = []
  for (const el of document.querySelectorAll(sel)) {
    const style = getComputedStyle(el)
    if (style.visibility === 'hidden' || style.display === 'none') continue
    const target = el.matches('input[type="checkbox"], input[type="radio"]') ? el.closest('label') || el : el
    const r = target.getBoundingClientRect()
    if (r.width === 0 || r.height === 0) continue
    // Parts of a scroll area that are off to the side are measured by size, not position.
    if (r.width < 44 || r.height < 44) {
      const name = (el.getAttribute('aria-label') || el.textContent || el.className || el.tagName).toString().trim().slice(0, 40)
      bad.push(`${el.tagName.toLowerCase()}.${(el.className || '').toString().split(' ')[0]} "${name}" ${Math.round(r.width)}x${Math.round(r.height)}`)
    }
  }
  return {
    bad,
    scroll: { page: document.documentElement.scrollWidth, window: window.innerWidth },
  }
}

async function main() {
  await new Promise((r) => server.listen(0, '127.0.0.1', r))
  const base = `http://127.0.0.1:${server.address().port}`
  const browser = await chromium.launch()
  const ctx = await browser.newContext({ viewport: { width: 360, height: 800 }, hasTouch: true, isMobile: true, deviceScaleFactor: 2 })
  const failures = []
  const shot = async (page, name) => {
    if (!shots) return
    mkdirSync(shots, { recursive: true })
    await page.screenshot({ path: join(shots, `${theme}-${name}.png`) })
  }
  const check = async (page, label) => {
    const m = await page.evaluate(measure)
    const wide = m.scroll.page > m.scroll.window
    console.log(`${wide || m.bad.length ? 'FAIL' : 'ok  '} ${label}: page ${m.scroll.page}px in ${m.scroll.window}px, ${m.bad.length} small control(s)`)
    if (wide) failures.push(`${label}: the page scrolls sideways (${m.scroll.page} > ${m.scroll.window})`)
    for (const b of m.bad) {
      console.log(`       ${b}`)
      failures.push(`${label}: ${b}`)
    }
  }
  for (const view of ['sheet', 'slides', 'dashboard', 'doc']) {
    const page = await ctx.newPage()
    const errors = []
    page.on('pageerror', (e) => errors.push(String(e)))
    await page.goto(`${base}/?view=${view}&theme=${theme}&density=${density}`)
    await page.waitForSelector('.su-view', { timeout: 10000 })
    await check(page, view)
    if (view === 'dashboard') {
      await page.locator('.su-svg').first().tap({ position: { x: 100, y: 100 } })
      const reading = await page.locator('.su-reading').first().innerText()
      if (/Tap the chart/.test(reading)) failures.push('dashboard: tapping the chart did not read a value')
      else console.log(`ok   dashboard tap reads: ${reading}`)
    }
    await shot(page, view)
    if (view === 'sheet') {
      // A tap picks a cell, a second tap on it opens the actions sheet.
      const cell = page.locator('td[role="gridcell"]').first()
      await cell.tap()
      const bar = await page.getByLabel('Formula', { exact: true }).inputValue()
      if (bar !== 'Sugar') failures.push(`sheet: the formula bar shows "${bar}" after tapping A2, not "Sugar"`)
      await cell.tap()
      await page.waitForSelector('dialog[open]', { timeout: 5000 })
      await check(page, 'sheet actions')
      await shot(page, 'sheet-actions')
      await page.keyboard.press('Escape')
      await page.getByRole('button', { name: 'View', exact: true }).tap()
      await page.waitForSelector('dialog[open]')
      await check(page, 'sheet view options')
      await shot(page, 'sheet-view-options')
    }
    if (view === 'slides') {
      await page.getByRole('button', { name: 'Present' }).tap()
      await page.waitForSelector('.su-present')
      await page.touchscreen.tap(330, 400)
      const seen = await page.locator('.su-present-n').innerText()
      if (!/Slide 2 of 3/.test(seen)) failures.push(`slides: a tap on the right side showed "${seen}", not slide 2`)
      await page.touchscreen.tap(20, 400)
      await check(page, 'slides present')
      await shot(page, 'slides-present')
    }
    if (view === 'doc') {
      await page.getByRole('button', { name: 'Outline' }).tap()
      await page.waitForSelector('dialog[open]')
      await check(page, 'doc outline')
      await shot(page, 'doc-outline')
    }
    for (const e of errors) failures.push(`${view}: page error ${e}`)
    await page.close()
  }
  await browser.close()
  server.close()
  if (failures.length) {
    console.error(`\n${failures.length} problem(s)`)
    process.exit(1)
  }
  console.log('\nall suite views pass')
}
main().catch((e) => {
  console.error(e)
  server.close()
  process.exit(1)
})
