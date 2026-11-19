import type { APIRoute, GetStaticPaths } from 'astro';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { MEDIA_DIR } from '../../../lib/docs-source.mjs';
const TYPES: Record<string, string> = { png: 'image/png', gif: 'image/gif' };

export const getStaticPaths: GetStaticPaths = () => readdirSync(MEDIA_DIR).map((file) => ({ params: { file } }));

export const GET: APIRoute = ({ params }) => {
  const file = String(params.file);
  const type = TYPES[file.split('.').pop() ?? ''];
  return new Response(readFileSync(join(MEDIA_DIR, file)), { headers: { 'Content-Type': type } });
};
