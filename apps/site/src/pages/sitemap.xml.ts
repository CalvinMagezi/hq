import type { APIRoute } from 'astro';
import { SITE_URL, nav } from '../data/site';

export const GET: APIRoute = () => {
  const urls = nav.map((n) => `  <url><loc>${new URL(n.href, SITE_URL).href}</loc></url>`).join('\n');
  const body = `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls}\n</urlset>\n`;
  return new Response(body, { headers: { 'Content-Type': 'application/xml' } });
};
