import { useEffect, useRef, useState } from 'react'
import * as pdfjsLib from 'pdfjs-dist'
import type { PDFDocumentProxy } from 'pdfjs-dist'
import workerUrl from 'pdfjs-dist/build/pdf.worker.mjs?url'
import { assetUrl } from '~/lib/vaultApi'
import { authHeaders } from '~/lib/hqAuth'
import { useAssetUrl } from '~/lib/useAssetUrl'

// Set worker src to load properly with Vite
pdfjsLib.GlobalWorkerOptions.workerSrc = workerUrl

export function PdfViewer({ path }: { path: string }) {
    const fileUrl = useAssetUrl(path)
    const [pdf, setPdf] = useState<PDFDocumentProxy | null>(null)
    const [pageNumber, setPageNumber] = useState(1)
    const [numPages, setNumPages] = useState<number | null>(null)
    const [scale, setScale] = useState(1.2)
    const [loading, setLoading] = useState(true)
    const [error, setError] = useState<string | null>(null)
    const canvasRef = useRef<HTMLCanvasElement>(null)

    useEffect(() => {
        let active = true
        setLoading(true)
        setError(null)
        const loadPdf = async () => {
            try {
                const loadingTask = pdfjsLib.getDocument({ url: assetUrl(path), httpHeaders: authHeaders() })
                const loadedPdf = await loadingTask.promise
                if (active) {
                    setPdf(loadedPdf)
                    setNumPages(loadedPdf.numPages)
                    setPageNumber(1)
                    setLoading(false)
                }
            } catch (e) {
                console.error('Failed to load PDF', e)
                if (active) {
                    setError('Failed to load PDF')
                    setLoading(false)
                }
            }
        }
        loadPdf()
        return () => { active = false }
    }, [path])

    useEffect(() => {
        let renderTask: any
        let active = true

        if (pdf && canvasRef.current && !loading) {
            const renderPage = async () => {
                try {
                    const page = await pdf.getPage(pageNumber)
                    const viewport = page.getViewport({ scale })
                    const canvas = canvasRef.current
                    if (!canvas) return
                    const context = canvas.getContext('2d')
                    if (!context) return

                    canvas.height = viewport.height
                    canvas.width = viewport.width

                    const renderContext = {
                        canvasContext: context,
                        viewport: viewport,
                        canvas: canvas
                    }

                    if (!active) return
                    renderTask = page.render(renderContext)
                    await renderTask.promise
                } catch (e) {
                    // ignore render cancellation
                    if (e instanceof Error && e.name !== 'RenderingCancelledException') {
                        console.error('Render error', e)
                    }
                }
            }
            renderPage()
        }

        return () => {
            active = false
            if (renderTask) renderTask.cancel()
        }
    }, [pdf, pageNumber, scale, loading])

    return (
        <div className="flex flex-col h-full w-full rounded-lg overflow-hidden border" style={{ background: 'var(--bg-elevated)', border: '1px solid var(--border)' }}>
            {/* Toolbar */}
            <div className="p-2 w-full flex justify-between items-center flex-shrink-0 border-b" style={{ borderColor: 'var(--border)', background: 'var(--bg-surface)' }}>
                <span className="text-xs font-mono truncate max-w-[200px]" style={{ color: 'var(--text-dim)' }}>
                    {path.split('/').pop()}
                </span>

                <div className="flex gap-4">
                    {/* Zoom Controls */}
                    <div className="flex gap-1 items-center bg-black/20 rounded p-1">
                        <button onClick={() => setScale(s => Math.max(0.5, s - 0.2))} className="px-2 text-xs rounded hover:bg-white/10">-</button>
                        <span className="text-[10px] font-mono w-10 text-center">{(scale * 100).toFixed(0)}%</span>
                        <button onClick={() => setScale(s => Math.min(3, s + 0.2))} className="px-2 text-xs rounded hover:bg-white/10">+</button>
                    </div>

                    {/* Page Controls */}
                    <div className="flex gap-1 items-center bg-black/20 rounded p-1">
                        <button
                            onClick={() => setPageNumber(p => Math.max(1, p - 1))}
                            disabled={pageNumber <= 1}
                            className="px-2 text-xs rounded hover:bg-white/10 disabled:opacity-30"
                        >
                            ◀
                        </button>
                        <span className="text-[10px] font-mono text-center px-2">
                            {pageNumber} / {numPages || '?'}
                        </span>
                        <button
                            onClick={() => setPageNumber(p => Math.min(numPages || 1, p + 1))}
                            disabled={pageNumber >= (numPages || 1)}
                            className="px-2 text-xs rounded hover:bg-white/10 disabled:opacity-30"
                        >
                            ▶
                        </button>
                    </div>

                    <a
                        href={fileUrl ?? undefined}
                        download
                        className="px-3 py-1 text-xs rounded flex items-center justify-center transition-colors"
                        style={{ background: 'var(--accent-blue)', color: '#fff' }}
                    >
                        Download
                    </a>
                </div>
            </div>

            {/* Canvas Container */}
            <div className="flex-1 overflow-auto w-full flex items-start justify-center p-4">
                {loading ? (
                    <div className="flex items-center justify-center h-full">
                        <span className="text-xs font-mono animate-pulse" style={{ color: 'var(--accent-green)' }}>Loading PDF...</span>
                    </div>
                ) : error ? (
                    <div className="flex items-center justify-center h-full text-xs font-mono" style={{ color: 'var(--accent-red)' }}>
                        {error}
                    </div>
                ) : (
                    <canvas ref={canvasRef} className="shadow-2xl bg-white" />
                )}
            </div>
        </div>
    )
}
