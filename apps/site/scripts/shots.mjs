// Full-page screenshots for visual review: `bun run shots <out-dir>`.
import { mkdirSync } from 'node:fs';
import { chromium } from 'playwright-core';
import { PAGES, serve } from './serve.mjs';

const out = process.argv[2] ?? '/tmp/hq-site-shots';
mkdirSync(out, { recursive: true });
const sizes = [{ name: 'desktop', width: 1280, height: 900 }, { name: 'phone', width: 390, height: 844 }];

const { server, base } = await serve();
const browser = await chromium.launch({ channel: 'chrome' });
// The site is dark only; the light run shows what a visitor who prefers light sees.
for (const theme of ['dark', 'light']) {
  for (const s of sizes) {
    const ctx = await browser.newContext({ viewport: { width: s.width, height: s.height }, colorScheme: theme });
    for (const path of PAGES.filter((p) => p !== '/404/')) {
      const page = await ctx.newPage();
      await page.goto(base + path, { waitUntil: 'networkidle' });
      // Scroll through once so lazy images load before the full-page capture.
      await page.evaluate(async () => {
        for (let y = 0; y < document.body.scrollHeight; y += 600) {
          window.scrollTo({ top: y, behavior: 'instant' });
          await new Promise((r) => setTimeout(r, 120));
        }
        window.scrollTo({ top: 0, behavior: 'instant' });
      });
      await page.waitForLoadState('networkidle');
      const slug = path === '/' ? 'home' : path.replaceAll('/', '');
      await page.screenshot({ path: `${out}/${slug}-${s.name}-${theme}.png`, fullPage: true });
      await page.close();
    }
    await ctx.close();
  }
}
await browser.close();
server.close();
console.log(`screenshots in ${out}`);
