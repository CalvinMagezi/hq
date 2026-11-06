// Bundles the control window's script and copies its two static files into ui/dist, which is
// what tauri.conf.json serves. No framework and no remote asset: the window runs under
// `script-src 'self'`.
import { cp, mkdir, rm } from 'node:fs/promises'

await rm('ui/dist', { recursive: true, force: true })
await mkdir('ui/dist', { recursive: true })
const built = await Bun.build({
  entrypoints: ['ui/src/main.ts'],
  outdir: 'ui/dist',
  target: 'browser',
  minify: true,
})
if (!built.success) {
  for (const log of built.logs) console.error(log)
  process.exit(1)
}
await cp('ui/index.html', 'ui/dist/index.html')
await cp('ui/style.css', 'ui/dist/style.css')
// The shared dark styles and the font files they name, from the web app, so both windows look alike.
await cp('../hq-web/src/styles/aligned-ui.css', 'ui/dist/aligned-ui.css')
await cp('../hq-web/public/fonts', 'ui/dist/fonts', { recursive: true })
// The HQ mark the stylesheet's `.a-mark` names.
await mkdir('ui/dist/icons', { recursive: true })
await cp('../hq-web/public/icons/hq-icon-192.png', 'ui/dist/icons/hq-icon-192.png')
