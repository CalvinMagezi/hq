import type { Metadata, Viewport } from 'next'
import type { ReactNode } from 'react'
import './globals.css'

export const metadata: Metadata = {
  title: 'Deploy Agent HQ on Hetzner',
  description: 'Create a private, self-updating Agent HQ server in your own Hetzner project.',
  robots: { index: true, follow: true },
  referrer: 'no-referrer',
}

export const viewport: Viewport = { width: 'device-width', initialScale: 1 }

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  )
}
