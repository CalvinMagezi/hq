import { defineConfig } from 'vite'
import { tanstackStart } from '@tanstack/react-start/plugin/vite'
import viteReact from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import path from 'node:path'
import fs from 'node:fs'

const certDir = path.resolve(__dirname, 'certs')
const tlsHost = process.env.TLS_HOST ?? ''
const certPath = tlsHost ? path.join(certDir, `${tlsHost}.crt`) : ''
const httpsConfig = certPath && fs.existsSync(certPath)
  ? {
      key: fs.readFileSync(path.join(certDir, `${tlsHost}.key`)),
      cert: fs.readFileSync(certPath),
    }
  : undefined

export default defineConfig({
  server: {
    port: 4747,
    strictPort: true,
    host: '0.0.0.0',
    allowedHosts: true,
    https: httpsConfig,
    proxy: {
      '/api': {
        target: 'http://localhost:5678',
      },
      '/ws': {
        target: 'http://localhost:5678',
        ws: true,
      },
    },
  },
  resolve: {
    alias: {
      '~': path.resolve(__dirname, 'src'),
    },
  },
  plugins: [
    // Static SPA: the build emits dist/client/index.html, which Rust hq-web serves for every client route.
    tanstackStart({ spa: { enabled: true, prerender: { outputPath: '/index' } } }),
    viteReact(),
    tailwindcss(),
  ],
})
