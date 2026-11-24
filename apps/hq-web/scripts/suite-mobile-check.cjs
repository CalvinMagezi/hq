#!/usr/bin/env node
// Loads the suite views (sheet, slides, dashboard, doc) from a static fixture at 360x800 with touch, and checks:
//   - no horizontal scroll on the page,
//   - every control a finger can press is at least 44px by 44px,
//   - nothing throws in the page,
//   - the review comments sheet adds, lists and resolves a comment, and no comment text is drawn in the note,
//   - a dashboard can be arranged (moved with the buttons, the arrow keys and a drag, and sized) and the note carries it,
//   - the area, stacked and scatter charts draw, and a kind that is not drawn says so.
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
const only = arg('only', null)

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
    // The note's own "Send to Chat" bar over selected words belongs to the note viewer, not the suite.
    if (el.closest('#hq-selection-toolbar')) continue
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


/** The Comments sheet: reads, adds on the block the person is at, and resolves. */
async function comments(page, failures, check, shot, view, want) {
  // An earlier step may have left a sheet open.
  if (await page.locator('dialog[open]').count()) {
    await page.keyboard.press('Escape')
    await page.waitForSelector('dialog[open]', { state: 'detached' })
  }
  const button = page.getByRole('button', { name: /^Comments/ })
  const badge = async () => (await page.locator('.su-cmt-btn .su-badge').count()) ? Number(await page.locator('.su-cmt-btn .su-badge').first().innerText()) : 0
  if ((await badge()) !== want.open) failures.push(`${view}: the Comments badge shows ${await badge()}, not ${want.open}`)
  await button.first().tap()
  await page.waitForSelector('dialog[open] .su-cmt-add')
  // The selection toolbar the note shows over selected words is not part of this sheet.
  await page.evaluate(() => window.getSelection()?.removeAllRanges())
  await page.waitForSelector('dialog[open] .su-cmts, dialog[open] p:has-text("No open comments")')
  await check(page, `${view} comments`)
  await shot(page, `${view}-comments`)
  const dialog = page.locator('dialog[open]')
  const chosen = await dialog.locator('select').first().evaluate((el) => el.selectedOptions[0]?.textContent || '')
  if (!want.anchor.test(chosen)) failures.push(`${view}: a new comment would go on "${chosen}"`)
  const had = await dialog.locator('.a-sheet-body > .su-cmts > .su-cmt').count()
  await dialog.locator('textarea').fill('Please check the wording here.')
  await dialog.getByRole('button', { name: 'Add comment' }).tap()
  await page.waitForFunction((n) => document.querySelectorAll('dialog[open] .a-sheet-body > .su-cmts > .su-cmt').length === n + 1, had, { timeout: 5000 }).catch(async (e) => {
    console.log('dialog says:', (await dialog.innerText()).replace(/\n+/g, ' | '))
    await shot(page, `${view}-comments-stuck`)
    throw e
  })
  if (!(await dialog.locator('.su-cmt', { hasText: 'Please check the wording here.' }).count())) failures.push(`${view}: the new comment is not listed`)
  if ((await badge()) !== want.open + 1) failures.push(`${view}: the badge shows ${await badge()} after adding, not ${want.open + 1}`)
  if (!/Please check/.test(await page.evaluate(() => window.__note()))) failures.push(`${view}: the note does not carry the new comment`)
  await shot(page, `${view}-comments-added`)
  if (want.open > 0) {
    await dialog.getByRole('button', { name: 'Resolve' }).first().tap()
    await page.waitForFunction((n) => document.querySelectorAll('dialog[open] .a-sheet-body > .su-cmts > .su-cmt').length === n, had, { timeout: 5000 }).catch(async (e) => {
      console.log('dialog says:', (await dialog.innerText()).replace(/\n+/g, ' | '))
      await shot(page, `${view}-comments-stuck`)
      throw e
    })
    if (!(await dialog.locator('.su-cmts-done').count())) failures.push(`${view}: resolved comments are not in a collapsed list`)
    await dialog.locator('.su-cmts-done summary').tap()
    await check(page, `${view} comments resolved`)
    await shot(page, `${view}-comments-resolved`)
  }
  await page.keyboard.press('Escape')
  await page.waitForSelector('dialog[open]', { state: 'detached' })
}

/** Arrange: move with the buttons, with the arrow keys and with a drag, and size; the note carries each. */
async function arrange(page, failures, check, shot) {
  const names = () => page.evaluate(() => [...document.querySelectorAll('.su-slot')].map((e) => (e.querySelector('.su-tile > .a-cap, .su-plot figcaption')?.textContent || '').trim()))
  const noteOrder = () => page.evaluate(() => [...window.__note().matchAll(/^```(tile|plot) .*?(?:label|title)="([^"]*)"/gm)].map((m) => m[2]))
  const same = (a, b) => JSON.stringify(a) === JSON.stringify(b)
  const before = await names()
  if (!same(before, ['Total sales', 'North', 'By month and region', 'Share'])) failures.push(`dashboard: the order shown is ${before}`)
  await page.getByRole('button', { name: 'Arrange' }).tap()
  await page.waitForSelector('.su-slot-bar')
  await check(page, 'dashboard arrange')
  await shot(page, 'dashboard-arrange')
  await page.getByRole('button', { name: 'Move Total sales later' }).tap()
  await page.waitForFunction(() => document.querySelector('.su-slot .su-tile > .a-cap')?.textContent === 'North', null, { timeout: 5000 }).catch(async (e) => {
    console.log('order', await names(), 'note', await noteOrder(), 'body', (await page.locator('body').innerText()).slice(0, 300).replace(/\n+/g, ' | '))
    throw e
  })
  if (!same(await noteOrder(), ['North', 'Total sales', 'By month and region', 'Share'])) failures.push(`dashboard: the note holds ${await noteOrder()} after Move later`)
  // The arrow keys on a grip move the block and focus stays with it.
  await page.getByRole('button', { name: /^Move Total sales:/ }).focus()
  await page.keyboard.press('ArrowUp')
  await page.waitForFunction(() => document.querySelector('.su-slot .su-tile > .a-cap')?.textContent === 'Total sales')
  const focused = await page.evaluate(() => document.activeElement?.getAttribute('aria-label') || '')
  if (!/^Move Total sales:/.test(focused)) failures.push(`dashboard: focus is on "${focused}" after the arrow key`)
  if (!same(await noteOrder(), ['Total sales', 'North', 'By month and region', 'Share'])) failures.push(`dashboard: the note holds ${await noteOrder()} after the arrow key`)
  // A drag of the second block onto the first.
  const gripBox = page.getByRole('button', { name: /^Move North:/ })
  await gripBox.scrollIntoViewIfNeeded()
  const grip = await gripBox.boundingBox()
  const target = await page.locator('.su-slot').first().boundingBox()
  await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2)
  await page.mouse.down()
  await page.mouse.move(grip.x + 10, grip.y - 40, { steps: 4 })
  await page.mouse.move(target.x + target.width / 2, target.y + target.height / 2, { steps: 8 })
  await shot(page, 'dashboard-dragging')
  await page.mouse.up()
  await page.waitForFunction(() => document.querySelector('.su-slot:first-child .su-tile > .a-cap')?.textContent === 'North', null, { timeout: 5000 })
  if (!same(await noteOrder(), ['North', 'Total sales', 'By month and region', 'Share'])) failures.push(`dashboard: the note holds ${await noteOrder()} after a drag`)
  // Size: a tile made full width, a plot made small.
  await page.getByLabel('Size of Total sales').selectOption('full')
  await page.waitForFunction(() => document.querySelectorAll('.su-slot.su-size-full').length === 1)
  if (!/label="Total sales" size=full/.test(await page.evaluate(() => window.__note()))) failures.push('dashboard: the note does not carry size=full')
  await page.getByLabel('Size of By month and region').selectOption('small')
  await page.waitForFunction(() => document.querySelectorAll('.su-slot.su-size-small').length === 2)
  await check(page, 'dashboard arranged')
  await shot(page, 'dashboard-arranged')
  await page.getByRole('button', { name: 'Done arranging' }).tap()
  if (await page.locator('.su-slot-bar').count()) failures.push('dashboard: the arrange bars stayed after Done')
  await shot(page, 'dashboard-done')
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
  for (const view of ['sheet', 'slides', 'dashboard', 'doc', 'charts'].filter((v) => !only || v === only)) {
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
      await page.keyboard.press('Escape')
      await page.waitForSelector('dialog[open]', { state: 'detached' })
      // Comment blocks are not drawn in the note: only a small marker after the block they are about.
      const body = await page.locator('.markdown-viewer').first().innerText()
      if (/Is this still true|Add a source/.test(body)) failures.push('doc: a comment was drawn in the note body')
      if ((await page.locator('.md-cmt-mark').count()) < 2) failures.push('doc: the blocks with comments have no marker')
      await shot(page, 'doc-markers')
      // The words the person selected decide where a new comment goes.
      await page.evaluate(() => {
        const p = [...document.querySelectorAll('.markdown-viewer p')].find((x) => /first paragraph/.test(x.textContent || ''))
        const range = document.createRange()
        range.selectNodeContents(p)
        const sel = window.getSelection()
        sel.removeAllRanges()
        sel.addRange(range)
      })
      await comments(page, failures, check, shot, 'doc', { open: 2, anchor: /paragraph: A first paragraph/ })
    }
    if (view === 'sheet' || view === 'slides') {
      // A fresh page: the earlier steps left a sheet or the presenter open.
      await page.reload()
      await page.waitForSelector('.su-view')
      if (view === 'slides') await page.locator('.su-thumb').nth(1).tap()
      const first = view === 'slides' ? { open: 2, anchor: /Plan/ } : { open: 0, anchor: /sheet/ }
      await comments(page, failures, check, shot, view, first)
    }
    if (view === 'dashboard') await arrange(page, failures, check, shot)
    if (view === 'charts') {
      const drawn = await page.locator('.su-svg').count()
      if (drawn !== 3) failures.push(`charts: ${drawn} charts drawn, not the 3 of area, stacked and scatter`)
      const said = await page.locator('.su-plot', { hasText: 'A kind not drawn' }).innerText()
      if (!/could not be drawn/.test(said)) failures.push(`charts: a kind that is not drawn said "${said}"`)
      for (const kind of ['polygon', 'rect.su-seg', 'path.su-dot']) if ((await page.locator(`.su-svg ${kind}`).count()) === 0) failures.push(`charts: nothing of ${kind} was drawn`)
      await shot(page, 'charts-top')
      await page.locator('.su-svg').nth(2).tap({ position: { x: 150, y: 100 } })
      console.log(`ok   charts scatter tap reads: ${await page.locator('.su-reading').nth(2).innerText()}`)
      await shot(page, 'charts')
    }
    for (const e of errors) failures.push(`${view}: page error ${e}`)
    await page.close()
  }
  await browser.close()
  server.close()
  if (failures.length) {
    console.error(`\n${failures.length} problem(s)`)
    for (const f of failures) console.error(`  - ${f}`)
    process.exit(1)
  }
  console.log('\nall suite views pass')
}
main().catch((e) => {
  console.error(e)
  server.close()
  process.exit(1)
})
