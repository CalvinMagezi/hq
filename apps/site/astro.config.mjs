import { defineConfig } from 'astro/config';

// Stylesheets stay external so the CSP can say style-src 'self' with no hashes.
export default defineConfig({
  site: 'https://agent-hq.online',
  output: 'static',
  trailingSlash: 'always',
  build: { format: 'directory', inlineStylesheets: 'never' },
  devToolbar: { enabled: false },
  // Highlighting would write inline style attributes, which the CSP forbids.
  markdown: { syntaxHighlight: false },
  // Compression drops the space between a line-wrapped word and the inline element after it
  // ("needs<a>bun</a>"), so prose would glue words together.
  compressHTML: false,
  // Lightning CSS folds `animation` plus `animation-timeline` into one shorthand that Chrome rejects,
  // which silently kills every scroll-driven animation. esbuild leaves the longhands alone.
  vite: { build: { cssMinify: 'esbuild' } },
});
