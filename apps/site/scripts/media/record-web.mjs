// Records a browser flow as a 1280x720 video, the same way every time.
//   bun scripts/media/record-web.mjs <steps.json> <out.webm>
// steps.json: { "start": "http://127.0.0.1:5678/", "steps": [ {"click":"text=Vault"}, {"fill":["#box","hello"]},
//   {"wait":1500}, {"goto":"/get/"} ] }. Point it at a demo instance, never at real data.
import { mkdirSync, readFileSync, renameSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { chromium } from 'playwright-core';

const [stepsFile, out] = process.argv.slice(2);
if (!stepsFile || !out) {
  console.error('usage: record-web.mjs <steps.json> <out.webm>');
  process.exit(1);
}
const flow = JSON.parse(readFileSync(stepsFile, 'utf8'));
const VIEWPORT = { width: 1280, height: 720 };
const dir = join(dirname(out), '.rec');
mkdirSync(dir, { recursive: true });

const browser = await chromium.launch({ channel: 'chrome' });
const context = await browser.newContext({ viewport: VIEWPORT, recordVideo: { dir, size: VIEWPORT } });
const page = await context.newPage();
await page.goto(flow.start);
for (const step of flow.steps) {
  if (step.goto) await page.goto(new URL(step.goto, flow.start).href);
  else if (step.click) await page.click(step.click);
  else if (step.fill) await page.fill(step.fill[0], step.fill[1]);
  else if (step.wait) await page.waitForTimeout(step.wait);
  else throw new Error(`unknown step ${JSON.stringify(step)}`);
}
const video = page.video();
await context.close();
renameSync(await video.path(), out);
await browser.close();
console.log(`wrote ${out}`);
