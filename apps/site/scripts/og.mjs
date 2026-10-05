// One-off generator for public/og.png (1200x630). The PNG is committed, so CI
// never needs a browser. Run manually after copy changes: `bun run og`.
import { readFileSync } from 'node:fs';
import { chromium } from 'playwright-core';

const ROOT = new URL('..', import.meta.url).pathname;
const icon = readFileSync(`${ROOT}public/icons/hq-icon-192.png`).toString('base64');

const html = `<!doctype html><meta charset="utf-8"><style>
  :root { --bg:#231f20; --ink:#fff; --muted:#d0d2d3; --blue:#00adee; --green:#22ff37; }
  * { box-sizing: border-box; }
  body { margin:0; width:1200px; height:630px; background:var(--bg); color:var(--ink);
    font-family: system-ui, -apple-system, 'Segoe UI', Roboto, Arial, sans-serif;
    padding:72px 80px; display:flex; flex-direction:column; justify-content:space-between; }
  .top { display:flex; align-items:center; gap:28px; }
  .top img { width:112px; height:112px; border-radius:20px; }
  h1 { margin:0; font-size:104px; line-height:1; letter-spacing:-0.01em; }
  .rule { height:3px; width:220px; background:linear-gradient(90deg,var(--blue),var(--green)); margin:36px 0 32px; }
  p { margin:0; font-size:44px; line-height:1.25; color:var(--muted); max-width:980px; }
  .foot { display:flex; justify-content:space-between; font-size:30px; color:var(--blue); font-weight:700; }
  .foot span:last-child { color:var(--muted); font-weight:400; }
</style>
<div>
  <div class="top"><img src="data:image/png;base64,${icon}" alt=""><h1>Agent HQ</h1></div>
  <div class="rule"></div>
  <p>A local-first AI agent hub. One Rust binary, a markdown vault, your machine, and your coding agents under watch.</p>
</div>
<div class="foot"><span>agent-hq.online</span><span>Open source, MIT, version 0.9.x</span></div>`;

const browser = await chromium.launch({ channel: 'chrome' });
const page = await browser.newPage({ viewport: { width: 1200, height: 630 }, deviceScaleFactor: 1 });
await page.setContent(html);
await page.screenshot({ path: `${ROOT}public/og.png`, type: 'png' });
await browser.close();
console.log('wrote public/og.png');
