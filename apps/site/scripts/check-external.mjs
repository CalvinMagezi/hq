// Checks that every outbound link in dist/ still resolves. Needs the network,
// so CI runs it as a non-blocking step: remote sites flake, and a dead link is
// a content fix, not a build failure.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';

const DIST = new URL('../dist', import.meta.url).pathname;
const OWN = 'https://agent-hq.online';
const TIMEOUT_MS = 20_000;
const CONCURRENCY = 4;

const walk = (d) => readdirSync(d).flatMap((n) => (statSync(join(d, n)).isDirectory() ? walk(join(d, n)) : [join(d, n)]));
const urls = new Set();
for (const f of walk(DIST).filter((p) => p.endsWith('.html'))) {
  for (const m of readFileSync(f, 'utf8').matchAll(/<a\b[^>]*\shref="(https:\/\/[^"#]+)/g)) {
    if (!m[1].startsWith(OWN)) urls.add(m[1]);
  }
}

async function probe(url) {
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), TIMEOUT_MS);
  try {
    const res = await fetch(url, { redirect: 'follow', signal: ctl.signal, headers: { 'user-agent': 'agent-hq-site-link-check' } });
    return res.status;
  } catch (e) {
    return `error: ${e.message}`;
  } finally {
    clearTimeout(timer);
  }
}

const queue = [...urls].sort();
const bad = [];
await Promise.all(
  Array.from({ length: CONCURRENCY }, async () => {
    while (queue.length) {
      const url = queue.shift();
      const status = await probe(url);
      if (typeof status !== 'number' || status >= 400) bad.push(`${status}  ${url}`);
    }
  })
);

console.log(`checked ${urls.size} outbound links`);
if (bad.length) {
  console.error('dead or unreachable links:\n  ' + bad.sort().join('\n  '));
  process.exit(1);
}
