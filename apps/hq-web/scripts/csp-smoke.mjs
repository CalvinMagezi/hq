// Loads the built app from a running HQ web server (which sends the real CSP) in headless
// Chromium and fails on any CSP violation or a blank page.
//   CSP_SMOKE_URL=http://127.0.0.1:5678 node scripts/csp-smoke.mjs
// Needs the optional `playwright` package and a browser; without them it skips (exit 0).
// Set CSP_SMOKE_REQUIRED=1 to make a skip a failure.
const url = process.env.CSP_SMOKE_URL ?? 'http://127.0.0.1:5678'
const required = process.env.CSP_SMOKE_REQUIRED === '1'

function skip(reason) {
  console.log(`csp-smoke: skipped (${reason})`)
  process.exit(required ? 1 : 0)
}

let chromium
try {
  ;({ chromium } = await import('playwright'))
} catch {
  skip('playwright is not installed')
}

let browser
try {
  browser = await chromium.launch()
} catch (e) {
  skip(`no Chromium available: ${String(e.message).split('\n')[0]}`)
}

const violations = []
const page = await browser.newPage()
await page.addInitScript(() => {
  document.addEventListener('securitypolicyviolation', (e) => {
    window.__cspViolations = window.__cspViolations ?? []
    window.__cspViolations.push(`${e.violatedDirective} blocked ${e.blockedURI || 'inline'}`)
  })
})
page.on('console', (m) => {
  if (/Content Security Policy|Refused to/i.test(m.text())) violations.push(m.text())
})
page.on('pageerror', (e) => violations.push(`page error: ${e.message}`))

await page.goto(url, { waitUntil: 'networkidle' })
await page.waitForTimeout(1500)
violations.push(...(await page.evaluate(() => window.__cspViolations ?? [])))
const shown = await page.evaluate(() => (document.getElementById('root') ?? document.body).innerText.trim().length)
await browser.close()

if (violations.length > 0) {
  console.error(`csp-smoke: FAILED, ${violations.length} problem(s):\n- ${violations.join('\n- ')}`)
  process.exit(1)
}
if (shown === 0) {
  console.error('csp-smoke: FAILED, the app rendered nothing')
  process.exit(1)
}
console.log('csp-smoke: ok')
