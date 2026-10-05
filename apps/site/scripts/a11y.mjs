// Browser checks: axe-core in both themes at desktop and phone width, CSP
// violations, horizontal overflow, keyboard focus visibility, theme toggle
// persistence. Needs Google Chrome (playwright-core drives it, no download).
// Run after `bun run build`: `bun run check:a11y`.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { chromium } from 'playwright-core';
import { PAGES, serve } from './serve.mjs';

const require = createRequire(import.meta.url);
const axeSource = readFileSync(require.resolve('axe-core/axe.min.js'), 'utf8');

const VIEWPORTS = [
  { name: 'desktop', width: 1280, height: 900 },
  { name: 'phone-360', width: 360, height: 740 },
  { name: 'phone-390', width: 390, height: 844 },
];
const THEMES = ['dark', 'light'];
const MAX_TAB_STOPS = 400;

const problems = [];
const note = (where, msg) => problems.push(`${where}: ${msg}`);

const { server, base } = await serve();
const browser = await chromium.launch({ channel: 'chrome' });

for (const theme of THEMES) {
  for (const vp of VIEWPORTS) {
    const ctx = await browser.newContext({ viewport: { width: vp.width, height: vp.height }, colorScheme: theme, reducedMotion: 'reduce' });
    for (const path of PAGES) {
      const where = `${path} [${theme}, ${vp.name}]`;
      const page = await ctx.newPage();
      const cspHits = [];
      page.on('console', (m) => /content security policy|refused to/i.test(m.text()) && cspHits.push(m.text()));
      page.on('requestfailed', (r) => note(where, `request failed ${r.url()}`));
      const outside = [];
      page.on('request', (r) => !r.url().startsWith(base) && outside.push(r.url()));
      await page.goto(base + path, { waitUntil: 'networkidle' });

      await page.evaluate(axeSource);
      const result = await page.evaluate(() => globalThis.axe.run(document, { runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa', 'best-practice'] } }));
      for (const v of result.violations) {
        note(where, `axe ${v.id} (${v.impact}): ${v.nodes.slice(0, 3).map((n) => n.target.join(' ')).join(' | ')}`);
      }

      const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
      if (overflow > 0) note(where, `page scrolls horizontally by ${overflow}px`);
      if (cspHits.length) note(where, `CSP violation: ${cspHits[0]}`);
      if (outside.length) note(where, `requests outside the origin: ${outside.join(', ')}`);

      const dataTheme = await page.evaluate(() => document.documentElement.getAttribute('data-theme'));
      if (dataTheme !== null) note(where, 'data-theme set before any choice');
      await page.close();
    }
    await ctx.close();
  }
}

// Keyboard: every focusable element shows a visible 3px outline, and the order reaches the footer.
{
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  for (const path of ['/', '/install/', '/compare/']) {
    const where = `${path} [keyboard]`;
    const page = await ctx.newPage();
    await page.goto(base + path);
    let reachedFooter = false;
    for (let i = 0; i < MAX_TAB_STOPS; i++) {
      await page.keyboard.press('Tab');
      const info = await page.evaluate(() => {
        const el = document.activeElement;
        if (!el || el === document.body) return null;
        const cs = getComputedStyle(el);
        return { tag: el.tagName, text: (el.textContent || el.getAttribute('aria-label') || '').trim().slice(0, 30), width: parseFloat(cs.outlineWidth), style: cs.outlineStyle, offset: parseFloat(cs.outlineOffset), inFooter: !!el.closest('footer') };
      });
      if (!info) break;
      if (info.style === 'none' || info.width < 3 || info.offset < 3) note(where, `no 3px ring with 3px offset on <${info.tag}> "${info.text}"`);
      if (info.inFooter) reachedFooter = true;
    }
    if (!reachedFooter) note(where, 'Tab order never reaches the footer');
    await page.close();
  }
  await ctx.close();
}

// Theme toggle persists across reloads and wins over the system preference.
{
  const ctx = await browser.newContext({ colorScheme: 'dark' });
  const page = await ctx.newPage();
  await page.goto(base + '/');
  await page.click('[data-theme-toggle]');
  await page.reload();
  const t = await page.evaluate(() => document.documentElement.getAttribute('data-theme'));
  const bg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
  if (t !== 'light' || bg !== 'rgb(255, 255, 255)') note('theme toggle', `expected persisted light theme, got ${t} / ${bg}`);
  await ctx.close();
}

// Copy buttons exist and say so when used.
{
  const ctx = await browser.newContext({ permissions: ['clipboard-read', 'clipboard-write'] });
  const page = await ctx.newPage();
  await page.goto(base + '/install/');
  const btn = page.locator('[data-copy="src-build"]');
  await btn.click();
  await page.waitForTimeout(300);
  const text = await btn.textContent();
  if (!/Copied|Press Ctrl/.test(text)) note('copy button', `no feedback, text is "${text}"`);
  await ctx.close();
}

await browser.close();
server.close();

if (problems.length) {
  console.error(`a11y check failed with ${problems.length} problem(s):\n` + problems.map((p) => `  - ${p}`).join('\n'));
  process.exit(1);
}
console.log(`a11y check passed: ${PAGES.length} pages x ${THEMES.length} themes x ${VIEWPORTS.length} viewports, keyboard, theme, copy.`);
