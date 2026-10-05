// Full-page screenshots for visual review: `bun run shots <out-dir>`.
import { mkdirSync } from 'node:fs';
import { chromium } from 'playwright-core';
import { PAGES, serve } from './serve.mjs';

const out = process.argv[2] ?? '/tmp/hq-site-shots';
mkdirSync(out, { recursive: true });
const sizes = [{ name: 'desktop', width: 1280, height: 900 }, { name: 'phone', width: 390, height: 844 }];

const { server, base } = await serve();
const browser = await chromium.launch({ channel: 'chrome' });
for (const theme of ['dark', 'light']) {
  for (const s of sizes) {
    const ctx = await browser.newContext({ viewport: { width: s.width, height: s.height }, colorScheme: theme });
    for (const path of PAGES.filter((p) => p !== '/404/')) {
      const page = await ctx.newPage();
      await page.goto(base + path, { waitUntil: 'networkidle' });
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
