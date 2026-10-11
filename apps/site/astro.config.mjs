import { defineConfig } from 'astro/config';

// Stylesheets stay external so the CSP can say style-src 'self' with no hashes.
export default defineConfig({
  site: 'https://agent-hq.online',
  output: 'static',
  trailingSlash: 'always',
  build: { format: 'directory', inlineStylesheets: 'never' },
  devToolbar: { enabled: false },
  // Compression drops the space between a line-wrapped word and the inline element after it
  // ("needs<a>bun</a>"), so prose would glue words together.
  compressHTML: false,
});
