// Static checks over dist/. Browser-free so CI stays cheap. Run after `bun run build`.
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { installChannels } from '../src/data/install-channels.ts';

const ROOT = new URL('..', import.meta.url).pathname;
const DIST = join(ROOT, 'dist');
const OWN_ORIGIN = 'https://agent-hq.online';

// Every outbound link must start with one of these. Add here only with a reason.
const OUTBOUND_ALLOW = [
  'https://github.com/CalvinMagezi/hq',
  'https://github.com/openclaw/openclaw',
  'https://github.com/NousResearch/hermes-agent',
  'https://docs.openclaw.ai',
  'https://hermes-agent.nousresearch.com/docs',
  'https://herdr.dev',
  'https://bun.sh',
  'https://rustup.rs',
  'https://jedisct1.github.io/minisign/',
  'https://tailscale.com/kb/',
];

const BANNED_WORDS = /\b(delve\w*|landscape\w*|tapestry|leverag\w*|robust\w*|seamless\w*|cutting-edge|innovative)\b/i;
const BANNED_PHRASES = [/it['’]s not just\b/i, /\bfinal thoughts\b/i, /\bin conclusion\b/i, /here['’]s the thing/i];
const EM_DASH = /—|&mdash;|&#8212;|&#x2014;/i;

const errors = [];
const fail = (page, msg) => errors.push(`${page}: ${msg}`);

function walk(dir) {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name);
    return statSync(p).isDirectory() ? walk(p) : [p];
  });
}

if (!existsSync(DIST)) {
  console.error('dist/ is missing. Run `bun run build` first.');
  process.exit(1);
}

const files = walk(DIST);
const pages = files.filter((f) => f.endsWith('.html'));
const rel = (f) => relative(DIST, f);

const attrs = (tag) => {
  const out = {};
  for (const m of tag.matchAll(/([a-zA-Z:-]+)(?:="([^"]*)")?/g)) out[m[1].toLowerCase()] = m[2] ?? '';
  return out;
};
const tags = (html, name) => [...html.matchAll(new RegExp(`<${name}\\b([^>]*)>`, 'gi'))].map((m) => ({ raw: m[0], a: attrs(m[1]) }));

const idsByPage = new Map();
for (const f of pages) {
  const html = readFileSync(f, 'utf8');
  idsByPage.set(f, new Set([...html.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1])));
}

function resolveInternal(path) {
  const clean = path.split('#')[0].split('?')[0];
  const candidates = clean.endsWith('/')
    ? [join(DIST, clean, 'index.html')]
    : [join(DIST, clean), join(DIST, clean, 'index.html')];
  return candidates.find((c) => existsSync(c) && statSync(c).isFile());
}

for (const file of pages) {
  const page = rel(file);
  const html = readFileSync(file, 'utf8');
  const noScripts = html.replace(/<script\b[\s\S]*?<\/script>/gi, '');

  // Typography and copy rules.
  if (EM_DASH.test(html)) fail(page, 'contains an em dash');
  const banned = noScripts.match(BANNED_WORDS);
  if (banned) fail(page, `banned word "${banned[0]}"`);
  for (const re of BANNED_PHRASES) if (re.test(noScripts)) fail(page, `banned phrase ${re}`);
  if (/\bnot just\b[^.]{0,80}\bit['’]s\b/i.test(noScripts)) fail(page, 'uses the "not just X, it is Y" construction');
  if (!installChannels.npx.available && /npx\s+agent-hq-cli/.test(html)) fail(page, 'presents npx agent-hq-cli while installChannels.npx.available is false');

  // CSP friendliness: nothing inline.
  if (/<style\b/i.test(html)) fail(page, 'inline <style> block');
  if (/\sstyle="/i.test(html)) fail(page, 'inline style attribute');
  if (/\son[a-z]+="/i.test(html)) fail(page, 'inline event handler attribute');
  // JSON-LD is inert data the CSP does not govern; every executable script must be a file.
  const executable = html.replace(/<script\b[^>]*type="application\/ld\+json"[^>]*>[\s\S]*?<\/script>/gi, '');
  for (const s of tags(executable, 'script')) if (!s.a.src) fail(page, 'inline <script>');
  if (/<script\b[^>]*>\s*\S/i.test(executable.replace(/<script\b[^>]*src=[^>]*><\/script>/gi, ''))) fail(page, 'inline script content');

  // Required document structure and meta.
  const need = (cond, what) => cond || fail(page, `missing ${what}`);
  need(/<html lang="en"/.test(html), 'html lang');
  need(/<title>[^<]{3,}<\/title>/.test(html), 'title');
  const metas = tags(html, 'meta');
  const metaBy = (key, val) => metas.find((m) => m.a[key] === val);
  const content = (key, val) => metaBy(key, val)?.a.content ?? '';
  const desc = content('name', 'description');
  need(desc.length >= 50 && desc.length <= 300, 'meta description (50 to 300 chars)');
  need(metaBy('name', 'viewport'), 'viewport');
  need(content('http-equiv', 'Content-Security-Policy').includes("script-src 'self'"), 'strict CSP meta');
  for (const p of ['og:title', 'og:description', 'og:image', 'og:url', 'og:type']) need(content('property', p), p);
  for (const n of ['twitter:card', 'twitter:image', 'twitter:title']) need(content('name', n), n);
  need(tags(html, 'link').some((l) => l.a.rel === 'canonical' && l.a.href.startsWith(OWN_ORIGIN)), 'canonical link on own origin');
  if (page !== '404.html') need(!metaBy('name', 'robots'), 'indexability (unexpected robots meta)');
  need((html.match(/<h1\b/g) ?? []).length === 1, 'exactly one h1');
  need(/<main\b[^>]*id="main"/.test(html) && /<header\b/.test(html) && /<footer\b/.test(html) && /<nav\b[^>]*aria-label/.test(html), 'landmarks (header, nav, main, footer)');
  need(/class="skip-link"/.test(html), 'skip link');

  // Heading order never skips a level.
  let last = 0;
  for (const m of html.matchAll(/<h([1-6])\b/g)) {
    const level = Number(m[1]);
    if (last && level > last + 1) fail(page, `heading jumps from h${last} to h${level}`);
    last = level;
  }

  // Duplicate ids, dangling references.
  const idList = [...html.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1]);
  const idSet = new Set(idList);
  for (const id of idList) if (idList.indexOf(id) !== idList.lastIndexOf(id)) fail(page, `duplicate id "${id}"`);
  for (const m of html.matchAll(/aria-(?:labelledby|describedby)="([^"]+)"/g)) {
    for (const ref of m[1].split(/\s+/)) if (!idSet.has(ref)) fail(page, `aria reference to missing id "${ref}"`);
  }
  for (const m of html.matchAll(/data-copy="([^"]+)"/g)) if (!idSet.has(m[1])) fail(page, `copy button targets missing id "${m[1]}"`);

  // Images: real alt text, and the file exists.
  for (const img of tags(html, 'img')) {
    if (!('alt' in img.a)) fail(page, `<img> without alt: ${img.a.src}`);
    else if (img.a.alt === '' && !/class="brand"[^>]*>\s*<img/.test(html)) fail(page, `empty alt on ${img.a.src}`);
    else if (img.a.alt.length > 0 && img.a.alt.length < 15) fail(page, `alt text too short on ${img.a.src}`);
    if (!img.a.width || !img.a.height) fail(page, `<img> without width and height: ${img.a.src}`);
  }
  for (const svg of tags(html, 'svg')) {
    const labelled = svg.a['aria-labelledby'] || svg.a['aria-label'];
    if (svg.a.role === 'img' && !labelled) fail(page, 'svg role=img without a name');
    if (!svg.a.role && svg.a['aria-hidden'] !== 'true') fail(page, 'svg neither hidden nor named');
  }
  for (const v of tags(html, 'video')) {
    if (/\bautoplay\b/.test(v.raw)) fail(page, 'video autoplays');
    if (!/\bcontrols\b/.test(v.raw)) fail(page, 'video without controls');
  }

  // Links and external requests.
  const refs = [];
  for (const [tag, attr] of [['a', 'href'], ['link', 'href'], ['img', 'src'], ['script', 'src'], ['source', 'src'], ['video', 'src'], ['video', 'poster'], ['iframe', 'src'], ['form', 'action']]) {
    for (const t of tags(html, tag)) if (t.a[attr] !== undefined) refs.push({ tag, attr, url: t.a[attr] });
  }
  for (const m of html.matchAll(/(?:srcset|data-poster|data-anim-src)="([^"]+)"/g)) refs.push({ tag: '*', attr: 'srcset', url: m[1].split(/[\s,]+/)[0] });
  for (const { tag, attr, url } of refs) {
    if (/^http:\/\//i.test(url)) fail(page, `insecure URL ${url}`);
    if (/^https:\/\//i.test(url)) {
      const own = url.startsWith(OWN_ORIGIN);
      if (tag === 'a' && !own && !OUTBOUND_ALLOW.some((p) => url.startsWith(p))) fail(page, `undocumented outbound link ${url}`);
      if (tag !== 'a' && !own) fail(page, `external request via <${tag} ${attr}>: ${url}`);
      continue;
    }
    if (url.startsWith('#')) {
      if (url.length > 1 && !idSet.has(url.slice(1))) fail(page, `broken fragment ${url}`);
      continue;
    }
    if (/^(mailto:|tel:|javascript:|data:)/i.test(url)) {
      fail(page, `disallowed scheme in ${url}`);
      continue;
    }
    if (url.startsWith('//')) {
      fail(page, `protocol-relative URL ${url}`);
      continue;
    }
    if (!url.startsWith('/')) {
      fail(page, `relative URL ${url} (use root-absolute paths)`);
      continue;
    }
    const target = resolveInternal(url);
    if (!target) {
      fail(page, `broken internal link ${url}`);
      continue;
    }
    if (tag === 'a' && target.endsWith('index.html') && !url.split('#')[0].endsWith('/') && url !== '/') fail(page, `page link without trailing slash ${url}`);
    const frag = url.split('#')[1];
    if (frag && target.endsWith('.html') && !idsByPage.get(target)?.has(frag)) fail(page, `broken fragment ${url}`);
  }
}

// Non-HTML assets must not reach out either.
for (const f of files.filter((p) => /\.(css|js|mjs)$/.test(p))) {
  const text = readFileSync(f, 'utf8');
  if (/@import|url\(\s*["']?https?:/i.test(text)) fail(rel(f), 'CSS imports or loads a remote resource');
  if (/\b(fetch|XMLHttpRequest|WebSocket|sendBeacon)\b/.test(text)) fail(rel(f), 'script uses a network API');
  if (/\bhttps?:\/\//.test(text)) fail(rel(f), 'script or style mentions a remote URL');
  if (EM_DASH.test(text)) fail(rel(f), 'contains an em dash');
}

// Site-level files.
const need = (cond, what) => cond || fail('site', what);
need(existsSync(join(DIST, 'CNAME')) && readFileSync(join(DIST, 'CNAME'), 'utf8').trim() === 'agent-hq.online', 'CNAME must contain agent-hq.online');
need(existsSync(join(DIST, 'robots.txt')) && /Sitemap: https:\/\/agent-hq\.online\/sitemap\.xml/.test(readFileSync(join(DIST, 'robots.txt'), 'utf8')), 'robots.txt with sitemap line');
need(existsSync(join(DIST, 'sitemap.xml')), 'sitemap.xml');
need(existsSync(join(DIST, 'og.png')), 'og.png');
need(existsSync(join(DIST, '404.html')), '404.html');
if (existsSync(join(DIST, 'sitemap.xml'))) {
  const sm = readFileSync(join(DIST, 'sitemap.xml'), 'utf8');
  for (const m of sm.matchAll(/<loc>([^<]+)<\/loc>/g)) {
    if (!m[1].startsWith(OWN_ORIGIN)) fail('sitemap.xml', `foreign URL ${m[1]}`);
    else if (!resolveInternal(m[1].slice(OWN_ORIGIN.length))) fail('sitemap.xml', `dead URL ${m[1]}`);
  }
}

// The install script embeds the release public key; it must match the repository's key file.
{
  const keyFile = join(import.meta.dirname, '..', '..', '..', 'release', 'minisign.pub');
  const repoKey = readFileSync(keyFile, 'utf8').split('\n').filter((l) => l && !l.startsWith('untrusted comment:'))[0];
  const script = readFileSync(join(import.meta.dirname, '..', 'public', 'install.sh'), 'utf8');
  const embedded = script.match(/^PUBKEY_LINE="([^"]+)"/m)?.[1];
  if (embedded !== repoKey) fail('install.sh', 'embedded public key does not match release/minisign.pub');
}

if (errors.length) {
  console.error(`check failed with ${errors.length} problem(s):\n` + errors.map((e) => `  - ${e}`).join('\n'));
  process.exit(1);
}
console.log(`check passed: ${pages.length} pages, ${files.length} files.`);
