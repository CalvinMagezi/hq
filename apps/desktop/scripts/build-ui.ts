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
