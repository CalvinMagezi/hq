// Reads the repo's own markdown (docs/*.md) for the site without copying it. A doc's first heading
// becomes its title, links point at /docs/ pages or at GitHub, images are served from /docs/media/,
// and a link to a file that does not exist stops the build, so the site cannot drift from the repo.
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { basename, dirname, join, relative, resolve, sep } from 'node:path';

// The build runs from apps/site. import.meta.url is useless here: the bundler moves this file.
export const REPO_ROOT = resolve(process.cwd(), '../..');
export const DOCS_DIR = join(REPO_ROOT, 'docs');
export const MEDIA_DIR = join(DOCS_DIR, 'media');
const REPO_BLOB = 'https://github.com/CalvinMagezi/hq/blob/main';
const PNG_HEADER_BYTES = 24;
const GIF_HEADER_BYTES = 10;
const LINK = /(!?)\[([^\]]*)\]\(([^)\s]+)((?:\s+"[^"]*")?)\)/g;

export const docSlug = (file) => basename(file).replace(/\.md$/i, '').toLowerCase().replace(/_/g, '-');

export function docFiles() {
  return readdirSync(DOCS_DIR).filter((f) => f.endsWith('.md')).map((f) => join(DOCS_DIR, f));
}

export function parseFrontMatter(text) {
  const match = text.match(/^---\n([\s\S]*?)\n---\n/);
  if (!match) return { data: {}, body: text };
  const data = {};
  for (const line of match[1].split('\n')) {
    const kv = line.match(/^([a-z]+):\s*(.*)$/);
    if (!kv) continue;
    const raw = kv[2].replace(/^"(.*)"$/, '$1');
    data[kv[1]] = /^\d+$/.test(raw) ? Number(raw) : raw;
  }
  return { data, body: text.slice(match[0].length) };
}

const published = new Set(docFiles().filter((f) => parseFrontMatter(readFileSync(f, 'utf8')).data.audience !== 'archive').map(docSlug));

function imageSize(path) {
  const bytes = readFileSync(path);
  if (bytes.subarray(1, 4).toString() === 'PNG' && bytes.length >= PNG_HEADER_BYTES) return [bytes.readUInt32BE(16), bytes.readUInt32BE(20)];
  if (bytes.subarray(0, 3).toString() === 'GIF' && bytes.length >= GIF_HEADER_BYTES) return [bytes.readUInt16LE(6), bytes.readUInt16LE(8)];
  throw new Error(`docs image ${path} is not a PNG or GIF, so its size cannot be read`);
}

function rewriteTarget(url, from, problems) {
  if (/^[a-z][a-z0-9+.-]*:/i.test(url) || url.startsWith('#') || url.startsWith('//')) return url;
  const [pathPart, hash] = url.split('#');
  const target = resolve(dirname(from), pathPart.split('?')[0]);
  if (!existsSync(target)) {
    problems.push(`link to a path that does not exist: ${url}`);
    return url;
  }
  const anchor = hash ? `#${hash}` : '';
  if (dirname(target) === DOCS_DIR && target.endsWith('.md') && published.has(docSlug(target))) return `/docs/${docSlug(target)}/${anchor}`;
  if (target.startsWith(MEDIA_DIR + sep)) return `/docs/media/${relative(MEDIA_DIR, target)}`;
  return `${REPO_BLOB}/${relative(REPO_ROOT, target)}${anchor}`;
}

function rewriteLine(line, from, problems) {
  // A link that starts inside inline code is an example, not a link.
  const code = [...line.matchAll(/`[^`]*`/g)].map((m) => [m.index, m.index + m[0].length]);
  return line.replace(LINK, (whole, bang, text, url, title, offset) => {
    if (code.some(([start, end]) => offset >= start && offset < end)) return whole;
    const next = rewriteTarget(url, from, problems);
    if (!bang) return `[${text}](${next}${title})`;
    if (!next.startsWith('/docs/media/')) return whole;
    const [width, height] = imageSize(join(MEDIA_DIR, next.slice('/docs/media/'.length)));
    return `<img src="${next}" alt="${text.replace(/"/g, '&quot;')}" width="${width}" height="${height}" loading="lazy" />`;
  });
}

// Returns the page title, the body with that heading removed, and its links and images rewritten.
export function prepareDoc(file) {
  const { data, body } = parseFrontMatter(readFileSync(file, 'utf8'));
  const problems = [];
  let title = '';
  let fenced = false;
  const lines = [];
  for (const line of body.split('\n')) {
    if (line.startsWith('```')) fenced = !fenced;
    if (!fenced && !title && line.startsWith('# ')) {
      title = line.slice(2).trim();
      continue;
    }
    // A page has one h1, the title. A later top-level heading in the source becomes a section.
    const text = !fenced && line.startsWith('# ') ? `#${line}` : line;
    lines.push(fenced ? line : rewriteLine(text, file, problems));
  }
  if (!title) problems.push('no first-level heading to use as the title');
  if (problems.length) throw new Error(`${relative(REPO_ROOT, file)}: ${problems.join('; ')}`);
  return { data: { ...data, title, file: basename(file) }, body: lines.join('\n').replace(/^\n+/, '') };
}
