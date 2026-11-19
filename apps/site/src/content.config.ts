import { defineCollection } from 'astro:content';
import type { Loader } from 'astro/loaders';
import { z } from 'astro/zod';
import { docFiles, docSlug, prepareDoc } from './lib/docs-source.mjs';

// The schema is the lint: a doc without a summary or an audience stops the build.
const schema = z.object({
  title: z.string(),
  file: z.string(),
  audience: z.enum(['start', 'owner', 'server', 'build', 'archive']),
  order: z.number(),
  summary: z.string().min(20).max(160),
  updated: z.coerce.date(),
});

// Scrollable code and tables must be reachable by keyboard, and a table header cell needs a name.
const accessible = (html: string) =>
  html
    .replace(/<pre>/g, '<pre tabindex="0">')
    .replace(/<table>/g, '<div class="table-scroll" role="region" aria-label="Table" tabindex="0"><table>')
    .replace(/<\/table>/g, '</table></div>')
    .replace(/<th>\s*<\/th>/g, '<th><span class="visually-hidden">Item</span></th>');

const repoDocs: Loader = {
  name: 'repo-docs',
  load: async ({ store, parseData, generateDigest, renderMarkdown }) => {
    store.clear();
    for (const file of docFiles()) {
      const { data, body } = prepareDoc(file);
      const id = docSlug(file);
      const rendered = await renderMarkdown(body);
      store.set({ id, data: await parseData({ id, data }), body, digest: generateDigest(body), rendered: { ...rendered, html: accessible(rendered.html) } });
    }
  },
};

export const collections = { docs: defineCollection({ loader: repoDocs, schema }) };
