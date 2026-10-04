import { useState, useRef, useEffect } from 'react'
import DOMPurify from 'dompurify'
import { blockRemoteImages } from '~/lib/remoteImages'
import { useAssetUrl } from '~/lib/useAssetUrl'

export function HtmlViewer({ content, path: filePath }: { content: string; path: string }) {
    const fileUrl = useAssetUrl(filePath)
    const [viewMode, setViewMode] = useState<'render' | 'source'>('render')
    const iframeRef = useRef<HTMLIFrameElement>(null)
    const [iframeHeight, setIframeHeight] = useState(600)

    // Inject a dark-friendly base style into the rendered HTML so it doesn't
    // flash white inside our dark UI. We wrap the user's HTML in a minimal reset.
    const wrappedHtml = `<!DOCTYPE html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<style>
  *, *::before, *::after { box-sizing: border-box; }
  html, body { margin: 0; padding: 0; }
  body {
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif;
    font-size: 14px; line-height: 1.65; color: #ddd;
    background: #0c0c16; padding: 24px 28px;
    overflow-x: hidden; word-break: break-word;
  }
  a { color: #4488ff; }
  h1,h2,h3,h4,h5,h6 { color: #e8e8f0; margin: 1.2em 0 0.4em; }
  h1 { font-size: 1.6em; } h2 { font-size: 1.35em; } h3 { font-size: 1.15em; }
  img { max-width: 100%; height: auto; border-radius: 6px; }
  pre, code { font-family: 'JetBrains Mono', 'Fira Code', monospace; font-size: 12px; }
  pre { background: rgba(255,255,255,0.04); border: 1px solid rgba(255,255,255,0.08); border-radius: 8px; padding: 14px; overflow-x: auto; }
  code { background: rgba(255,255,255,0.06); padding: 2px 5px; border-radius: 3px; }
  pre code { background: none; padding: 0; }
  table { border-collapse: collapse; width: 100%; margin: 1em 0; }
  th, td { border: 1px solid rgba(255,255,255,0.1); padding: 8px 12px; text-align: left; }
  th { background: rgba(255,255,255,0.04); color: #00ffa3; font-size: 11px; text-transform: uppercase; letter-spacing: 0.04em; }
  tr:nth-child(even) { background: rgba(255,255,255,0.02); }
  blockquote { border-left: 3px solid #ffb300; margin: 1em 0; padding: 0.5em 1em; color: #999; background: rgba(255,179,0,0.04); border-radius: 0 6px 6px 0; }
  hr { border: none; border-top: 1px solid rgba(255,255,255,0.08); margin: 2em 0; }
  ul, ol { padding-left: 24px; }
  li { margin-bottom: 4px; }
</style></head><body>${blockRemoteImages(DOMPurify.sanitize(content))}</body></html>`

    // Auto-resize iframe to fit content
    useEffect(() => {
        if (viewMode !== 'render') return
        const timer = setInterval(() => {
            try {
                const doc = iframeRef.current?.contentDocument
                if (doc?.body) {
                    const h = doc.body.scrollHeight + 40
                    if (h !== iframeHeight && h > 100) setIframeHeight(h)
                }
            } catch { /* cross-origin guard */ }
        }, 300)
        return () => clearInterval(timer)
    }, [viewMode, iframeHeight])

    return (
        <div>
            {/* Toolbar */}
            <div
                className="flex items-center justify-between mb-3 gap-2"
            >
                <div className="flex items-center gap-2">
                    <span
                        className="text-[10px] font-mono px-2 py-1 rounded-lg font-bold tracking-wider"
                        style={{ background: 'rgba(255,68,68,0.1)', color: 'var(--accent-red)' }}
                    >
                        HTML
                    </span>

                    {/* Render / Source toggle */}
                    <div
                        className="flex rounded-xl p-0.5"
                        style={{ background: 'rgba(255,255,255,0.03)', border: '1px solid rgba(255,255,255,0.04)' }}
                    >
                        <button
                            onClick={() => setViewMode('render')}
                            className="px-2.5 py-1 text-[9px] tracking-wider uppercase font-mono font-bold transition-all rounded-lg"
                            style={{
                                color: viewMode === 'render' ? 'var(--accent-green)' : 'var(--text-dim)',
                                background: viewMode === 'render' ? 'rgba(0,255,163,0.08)' : 'transparent',
                            }}
                        >
                            Render
                        </button>
                        <button
                            onClick={() => setViewMode('source')}
                            className="px-2.5 py-1 text-[9px] tracking-wider uppercase font-mono font-bold transition-all rounded-lg"
                            style={{
                                color: viewMode === 'source' ? 'var(--accent-green)' : 'var(--text-dim)',
                                background: viewMode === 'source' ? 'rgba(0,255,163,0.08)' : 'transparent',
                            }}
                        >
                            Source
                        </button>
                    </div>
                </div>

                <a
                    href={fileUrl ?? undefined}
                    download
                    className="px-3 py-1.5 rounded-xl text-xs font-mono transition-all"
                    style={{ background: 'rgba(255,255,255,0.04)', color: 'var(--text-dim)', border: '1px solid rgba(255,255,255,0.08)' }}
                >
                    Download
                </a>
            </div>

            {/* Content */}
            {viewMode === 'render' ? (
                <div
                    className="rounded-xl overflow-hidden"
                    style={{ border: '1px solid rgba(255,255,255,0.06)' }}
                >
                    <iframe
                        ref={iframeRef}
                        srcDoc={wrappedHtml}
                        sandbox="allow-same-origin"
                        title="HTML Preview"
                        className="w-full border-0"
                        style={{
                            height: `${iframeHeight}px`,
                            maxHeight: '80vh',
                            background: '#0c0c16',
                        }}
                    />
                </div>
            ) : (
                <div
                    className="rounded-xl overflow-auto text-xs font-mono leading-relaxed p-4"
                    style={{
                        background: 'rgba(22,22,38,0.6)',
                        border: '1px solid rgba(0,255,163,0.15)',
                        color: 'var(--text-primary)',
                        maxHeight: '80vh',
                        whiteSpace: 'pre-wrap',
                        wordBreak: 'break-word',
                    }}
                >
                    {content}
                </div>
            )}
        </div>
    )
}
