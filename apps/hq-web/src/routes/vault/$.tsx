import { createFileRoute, Link } from '@tanstack/react-router'
import { lazy, Suspense, useState, useCallback, useEffect, useMemo } from 'react'
import { marked } from 'marked'
import DOMPurify from 'dompurify'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { fetchNoteExport, togglePinNote } from '~/lib/vaultApi'
import { exportFileName, exportOptionsFor, NOTE_EXPORT_OPTIONS, type NoteExportFormat } from '~/lib/noteExport'
import { HqHttpError } from '~/lib/hqAuth'
import { noteQuery, vaultKeys } from '~/lib/queries'
import type { DirEntry } from '~/lib/vaultApi'
import { fileIcon } from '~/components/vault/fileIcon'
import { MarkdownViewer } from '~/components/MarkdownViewer'
import { ImageViewer } from '~/components/ImageViewer'
import { NoteEditor } from '~/components/NoteEditor'
import { useHQStore } from '~/store/hqStore'
import { CopyPathMenu, type CopyMenuState } from '~/components/CopyPathMenu'
import { ExportMenu, type ExportMenuState } from '~/components/ExportMenu'
import { usePersistedState } from '~/lib/usePersistedState'

const MD_ZOOM_MIN = 0.7
const MD_ZOOM_MAX = 1.6
const MD_ZOOM_STEP = 0.1


// Lazy-load heavy viewer components to reduce initial bundle
const PdfViewer = lazy(() => import('~/components/PdfViewer').then(m => ({ default: m.PdfViewer })))
const CodeViewer = lazy(() => import('~/components/CodeViewer').then(m => ({ default: m.CodeViewer })))
const DocxViewer = lazy(() => import('~/components/DocxViewer').then(m => ({ default: m.DocxViewer })))
const SpreadsheetViewer = lazy(() => import('~/components/SpreadsheetViewer').then(m => ({ default: m.SpreadsheetViewer })))
const OfficeFileCard = lazy(() => import('~/components/OfficeFileCard').then(m => ({ default: m.OfficeFileCard })))
const HtmlViewer = lazy(() => import('~/components/HtmlViewer').then(m => ({ default: m.HtmlViewer })))

export const Route = createFileRoute('/vault/$')({
    component: VaultFileRoute,
    // Data comes from the saved copy first, then the Rust API, so render client-side.
    ssr: false,
})

/** Shows the saved copy of a note at once; mounts the view only once data exists. */
function VaultFileRoute() {
    const filePath = Route.useParams()._splat || ''
    const note = useQuery(noteQuery(filePath))
    if (!note.data) {
        return (
            <div className="h-full flex items-center justify-center text-xs font-mono" style={{ color: 'var(--text-dim)' }}>
                {note.isError ? 'Could not load this note, and no saved copy is on this device.' : 'Loading...'}
            </div>
        )
    }
    return (
        <VaultFileView
            key={filePath}
            filePath={filePath}
            content={note.data.content}
            isDir={note.data.isDir}
            dirEntries={note.data.dirEntries ?? []}
        />
    )
}

function DirView({ dirPath, entries }: { dirPath: string; entries: DirEntry[] }) {
    const name = dirPath.split('/').pop() ?? dirPath
    return (
        <div className="p-6">
            <div className="flex items-center gap-2 mb-5">
                <span className="text-lg">📁</span>
                <h1 className="text-sm font-mono font-bold" style={{ color: 'var(--text-primary)' }}>{name}</h1>
                <span className="text-[10px] font-mono" style={{ color: 'var(--text-dim)', opacity: 0.4 }}>{entries.length} items</span>
            </div>
            <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-2">
                {entries.map(e => (
                    <Link
                        key={e.path}
                        to="/vault/$"
                        params={{ _splat: e.path }}
                        className="flex items-center gap-2.5 px-3 py-2.5 rounded-xl transition-all"
                        style={{ background: 'rgba(255,255,255,0.03)', border: '1px solid rgba(255,255,255,0.05)' }}
                        onMouseEnter={ev => { ev.currentTarget.style.background = 'rgba(0,173,238,0.06)'; ev.currentTarget.style.borderColor = 'rgba(0,173,238,0.15)' }}
                        onMouseLeave={ev => { ev.currentTarget.style.background = 'rgba(255,255,255,0.03)'; ev.currentTarget.style.borderColor = 'rgba(255,255,255,0.05)' }}
                    >
                        <span className="text-[14px] flex-shrink-0">{e.isDir ? '📁' : fileIcon(e.name)}</span>
                        <span className="text-[11px] font-mono truncate" style={{ color: e.isDir ? 'var(--text-primary)' : 'var(--text-dim)' }}>
                            {e.name.replace(/\.md$/, '')}
                        </span>
                    </Link>
                ))}
            </div>
        </div>
    )
}

function VaultFileView({ filePath, content, isDir, dirEntries }: {
    filePath: string
    content: string
    isDir: boolean
    dirEntries: DirEntry[]
}) {
    const { setGlobalChatOpen, setChatDraft } = useHQStore()
    const queryClient = useQueryClient()

    const ext = filePath.split('.').pop()?.toLowerCase() ?? ''
    const isMd = ext === 'md'

    // All hooks must be called unconditionally before any early returns
    const initialPinned = isMd && /^---[\s\S]*?^pinned:\s*true/m.test(content)
    const [isPinned, setIsPinned] = useState(initialPinned)
    const [pinning, setPinning] = useState(false)
    const [editing, setEditing] = useState(false)
    const [exporting, setExporting] = useState<NoteExportFormat | null>(null)
    const [exportMenu, setExportMenu] = useState<ExportMenuState | null>(null)
    const closeExportMenu = useCallback(() => setExportMenu(null), [])
    // Table formats are only offered for a note that has a table.
    const exportOptions = useMemo(() => exportOptionsFor(content), [content])
    const [copyMenu, setCopyMenu] = useState<CopyMenuState | null>(null)
    const closeCopyMenu = useCallback(() => setCopyMenu(null), [])

    const [mdZoom, setMdZoom] = usePersistedState(
        'hq-md-zoom',
        1,
        (v) => typeof v === 'number' && v >= MD_ZOOM_MIN && v <= MD_ZOOM_MAX,
    )

    const setMdZoomPersisted = useCallback(
        (z: number) => setMdZoom(Math.max(MD_ZOOM_MIN, Math.min(MD_ZOOM_MAX, +z.toFixed(1)))),
        [setMdZoom],
    )

    useEffect(() => {
        if (!isMd || editing) return
        const handleKeyDown = (e: KeyboardEvent) => {
            if (!e.metaKey && !e.ctrlKey) return
            if (e.key === '=' || e.key === '+') {
                e.preventDefault()
                setMdZoomPersisted(mdZoom + MD_ZOOM_STEP)
            } else if (e.key === '-') {
                e.preventDefault()
                setMdZoomPersisted(mdZoom - MD_ZOOM_STEP)
            } else if (e.key === '0') {
                e.preventDefault()
                setMdZoomPersisted(1)
            }
        }
        window.addEventListener('keydown', handleKeyDown)
        return () => window.removeEventListener('keydown', handleKeyDown)
    }, [isMd, editing, mdZoom, setMdZoomPersisted])


    if (isDir) {
        return (
            <div className="h-full overflow-y-auto">
                <DirView dirPath={filePath} entries={dirEntries} />
            </div>
        )
    }

    const handleTogglePin = async () => {
        setPinning(true)
        try {
            await togglePinNote(filePath, !isPinned)
            setIsPinned(!isPinned)
            queryClient.invalidateQueries({ queryKey: vaultKeys.pinned })
        } finally {
            setPinning(false)
        }
    }

    const handleConvertToTask = () => {
        setChatDraft(
            `Convert this vault note into a task: ${filePath}. Use task_create_from_note to place it in the right Space > Folder > List, creating a new Space only if nothing fits.`,
        )
        setGlobalChatOpen(true)
    }

    // Server-rendered export in any of the offered formats. Touch devices get the share sheet
    // (WhatsApp, email, Drive); desktops get a download.
    const handleExport = async (format: NoteExportFormat) => {
        if (exporting) return
        setExportMenu(null)
        setExporting(format)
        try {
            const { blob, extension } = await fetchNoteExport(filePath, format)
            const name = exportFileName(filePath, extension)
            const file = new File([blob], name, { type: blob.type || 'application/octet-stream' })
            const touch = window.matchMedia?.('(pointer: coarse)').matches
            if (touch && navigator.canShare?.({ files: [file] })) {
                try {
                    await navigator.share({ files: [file], title: name })
                } catch (err) {
                    if (!(err instanceof DOMException && err.name === 'AbortError')) throw err
                }
                return
            }
            const url = URL.createObjectURL(blob)
            const a = document.createElement('a')
            a.href = url
            a.download = name
            document.body.appendChild(a)
            a.click()
            a.remove()
            setTimeout(() => URL.revokeObjectURL(url), 10_000)
        } catch (err) {
            if (format === 'pdf' && err instanceof HqHttpError && err.status === 503) {
                // The server's older PDF engine is not installed: print from the browser instead.
                await handlePrintPDF()
            } else {
                const label = NOTE_EXPORT_OPTIONS.find((o) => o.format === format)?.label ?? format
                window.alert(err instanceof Error ? err.message : `Could not export this note as ${label}.`)
            }
        } finally {
            setExporting(null)
        }
    }

    // Browser print window: the fallback when the server has no PDF engine.
    const handlePrintPDF = async () => {
        const filename = filePath.split('/').pop()?.replace(/\.md$/, '') ?? 'note'

        // Strip YAML frontmatter before rendering
        const body = content.replace(/^---[\s\S]*?---\n?/, '')
        let html: string
        try { html = await marked.parse(body) } catch {
            window.alert('Could not convert this note to HTML for printing.')
            return
        }
        // Escape filename and sanitize html to prevent XSS in the print window
        const escapeHtml = (s: string) => s.replace(/[<>&"']/g, c => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;', '"': '&quot;', "'": '&#39;' }[c] ?? c))
        const safeFilename = escapeHtml(filename)
        const safeHtml = DOMPurify.sanitize(html, { USE_PROFILES: { html: true } })

        const win = window.open('', '_blank', 'width=900,height=700')
        if (!win) return

        win.document.write(`<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>${safeFilename}</title>
<style>
  *, *::before, *::after { box-sizing: border-box; }
  body {
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
    font-size: 14px;
    line-height: 1.7;
    color: #1a1a2e;
    max-width: 760px;
    margin: 0 auto;
    padding: 48px 40px;
  }
  h1 { font-size: 26px; font-weight: 700; margin: 0 0 24px; border-bottom: 2px solid #e2e8f0; padding-bottom: 12px; }
  h2 { font-size: 20px; font-weight: 600; margin: 32px 0 12px; }
  h3 { font-size: 16px; font-weight: 600; margin: 24px 0 8px; }
  p { margin: 0 0 16px; }
  a { color: #3b82f6; }
  code {
    font-family: 'JetBrains Mono', 'Fira Code', monospace;
    font-size: 12px;
    background: #f1f5f9;
    padding: 2px 6px;
    border-radius: 4px;
  }
  pre {
    background: #f8fafc;
    border: 1px solid #e2e8f0;
    border-radius: 6px;
    padding: 16px;
    overflow-x: auto;
    margin: 0 0 16px;
  }
  pre code { background: none; padding: 0; font-size: 12px; }
  blockquote {
    border-left: 3px solid #94a3b8;
    margin: 0 0 16px;
    padding: 4px 0 4px 16px;
    color: #64748b;
  }
  table { width: 100%; border-collapse: collapse; margin: 0 0 16px; font-size: 13px; }
  th, td { border: 1px solid #e2e8f0; padding: 8px 12px; text-align: left; }
  th { background: #f8fafc; font-weight: 600; }
  ul, ol { margin: 0 0 16px; padding-left: 24px; }
  li { margin-bottom: 4px; }
  hr { border: none; border-top: 1px solid #e2e8f0; margin: 24px 0; }
  .meta { font-size: 11px; color: #94a3b8; margin-bottom: 32px; }
  @media print {
    body { padding: 0; max-width: 100%; }
    a { color: #1a1a2e; text-decoration: none; }
  }
</style>
</head>
<body>
<h1>${safeFilename}</h1>
<p class="meta">${escapeHtml(filePath)} · ${new Date().toLocaleDateString('en-GB', { day: 'numeric', month: 'long', year: 'numeric' })}</p>
${safeHtml}
<script>window.onload = () => { window.print(); }<\/script>
</body>
</html>`)
        win.document.close()
    }

    // File rendering
    let viewer
    if (filePath.endsWith('.pdf')) {
        viewer = <PdfViewer path={filePath} />
    } else if (/^(png|jpe?g|gif|webp|svg)$/.test(ext)) {
        viewer = <ImageViewer path={filePath} />
    } else if (ext === 'md') {
        if (editing) {
            viewer = (
                <NoteEditor
                    content={content}
                    filePath={filePath}
                    onSaved={() => queryClient.invalidateQueries({ queryKey: vaultKeys.note(filePath) })}
                />
            )
        } else {
            viewer = <MarkdownViewer content={content} activePath={filePath} zoom={mdZoom} />
        }
    } else if (ext === 'docx') {
        viewer = <DocxViewer path={filePath} />
    } else if (ext === 'xlsx' || ext === 'xls') {
        viewer = <SpreadsheetViewer path={filePath} />
    } else if (ext === 'pptx') {
        viewer = <OfficeFileCard path={filePath} />
    } else if (ext === 'html' || ext === 'htm') {
        viewer = <HtmlViewer content={content} path={filePath} />
    } else {
        viewer = <CodeViewer content={content} path={filePath} />
    }

    const filename = filePath.split('/').pop()

    return (
        <div className="h-full flex flex-col relative">
            {/* Action bar — sticky, compact, action-focused (path shown in bottom breadcrumb) */}
            <div
                className="flex items-center justify-between px-3 py-1.5 flex-shrink-0 sticky top-0 z-10 glass-heavy"
                style={{ borderBottom: '1px solid rgba(255,255,255,0.05)', minHeight: '36px' }}
            >
                <span className="text-[11px] font-mono font-bold truncate flex-1 mr-2" style={{ color: 'var(--text-primary)' }}>
                    {filename?.replace(/\.md$/, '')}
                </span>

                <div className="flex items-center gap-1 flex-shrink-0">
                    {isMd && (
                        <button
                            onClick={() => setEditing(!editing)}
                            className="flex items-center gap-1 px-2 py-1 rounded-lg text-[10px] font-mono font-bold transition-all"
                            style={{
                                color: editing ? 'var(--accent-violet)' : 'var(--text-dim)',
                                background: editing ? 'rgba(167,139,250,0.1)' : 'rgba(255,255,255,0.04)',
                                border: editing ? '1px solid rgba(167,139,250,0.2)' : '1px solid rgba(255,255,255,0.06)',
                            }}
                        >
                            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                                {editing
                                    ? <><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></>
                                    : <><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/></>
                                }
                            </svg>
                            {editing ? 'View' : 'Edit'}
                        </button>
                    )}
                    {isMd && (
                        <button
                            onClick={handleTogglePin}
                            disabled={pinning}
                            className="flex items-center justify-center w-7 h-7 rounded-lg transition-all"
                            style={{
                                color: isPinned ? 'var(--accent-amber)' : 'var(--text-dim)',
                                background: isPinned ? 'rgba(255,179,0,0.08)' : 'rgba(255,255,255,0.04)',
                                border: isPinned ? '1px solid rgba(255,179,0,0.15)' : '1px solid rgba(255,255,255,0.06)',
                            }}
                            title={isPinned ? 'Unpin' : 'Pin'}
                        >
                            <svg width="11" height="11" viewBox="0 0 24 24" fill={isPinned ? 'currentColor' : 'none'} stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                                <path d="M21 10c0 7-9 13-9 13s-9-6-9-13a9 9 0 0 1 18 0z"/><circle cx="12" cy="10" r="3"/>
                            </svg>
                        </button>
                    )}
                    {isMd && !editing && (
                        <div className="flex gap-1 items-center bg-black/20 rounded-lg p-0.5 border border-white/5 mr-1">
                            <button
                                onClick={() => setMdZoomPersisted(mdZoom - MD_ZOOM_STEP)}
                                disabled={mdZoom <= MD_ZOOM_MIN}
                                className="px-1.5 py-0.5 text-[10px] font-mono rounded hover:bg-white/10 disabled:opacity-30 transition-all"
                                style={{ color: 'var(--text-dim)' }}
                                title="Decrease text size (Cmd/Ctrl + -)"
                            >A-</button>
                            <span className="text-[9px] font-mono w-7 text-center" style={{ color: 'var(--text-dim)' }}>
                                {Math.round(mdZoom * 100)}%
                            </span>
                            <button
                                onClick={() => setMdZoomPersisted(mdZoom + MD_ZOOM_STEP)}
                                disabled={mdZoom >= MD_ZOOM_MAX}
                                className="px-1.5 py-0.5 text-[10px] font-mono rounded hover:bg-white/10 disabled:opacity-30 transition-all"
                                style={{ color: 'var(--text-dim)' }}
                                title="Increase text size (Cmd/Ctrl + +)"
                            >A+</button>
                            {mdZoom !== 1 && (
                                <button
                                    onClick={() => setMdZoomPersisted(1)}
                                    className="px-1 text-[9px] font-mono rounded hover:bg-white/10 text-amber-400/70 hover:text-amber-400 transition-all"
                                    title="Reset zoom (Cmd/Ctrl + 0)"
                                >↺</button>
                            )}
                        </div>
                    )}
                    {isMd && (
                        <button
                            onClick={(e) => {
                                const rect = e.currentTarget.getBoundingClientRect()
                                setExportMenu({ x: rect.right - 240, y: rect.bottom + 6 })
                            }}
                            disabled={exporting !== null}
                            className="flex items-center justify-center gap-1 h-7 px-2 rounded-lg transition-all disabled:opacity-40"
                            style={{ color: 'var(--text-dim)', background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.06)' }}
                            title={exporting ? 'Exporting…' : 'Export as PDF, Word, HTML, Excel and more'}
                            aria-haspopup="menu"
                            aria-expanded={exportMenu !== null}
                        >
                            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                                <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/>
                            </svg>
                            <span className="hidden sm:inline text-[10px] font-mono">{exporting ? 'Exporting…' : 'Export'}</span>
                        </button>
                    )}
                    {isMd && (
                        <button
                            onClick={handleConvertToTask}
                            className="hidden sm:flex items-center justify-center w-7 h-7 rounded-lg transition-all"
                            style={{ color: 'var(--text-dim)', background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.06)' }}
                            title="Convert to task"
                        >
                            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                                <path d="M9 11l3 3L22 4"/><path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11"/>
                            </svg>
                        </button>
                    )}
                    <button
                        onClick={(e) => {
                            const rect = e.currentTarget.getBoundingClientRect()
                            setCopyMenu({ relPath: filePath, x: rect.right - 220, y: rect.bottom + 6 })
                        }}
                        className="flex items-center justify-center w-7 h-7 rounded-lg transition-all"
                        style={{ color: 'var(--text-dim)', background: 'rgba(255,255,255,0.04)', border: '1px solid rgba(255,255,255,0.06)' }}
                        title="Copy path"
                    >
                        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                            <rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>
                        </svg>
                    </button>
                </div>
            </div>

            {/* Content Area */}
            <div className="flex-1 min-h-0 overflow-y-auto overflow-x-hidden w-full">
                <Suspense fallback={
                    <div className="flex items-center justify-center py-12">
                        <span className="text-sm font-mono animate-pulse" style={{ color: 'var(--text-dim)' }}>Loading viewer...</span>
                    </div>
                }>
                    <div className="max-w-[860px] mx-auto p-4 sm:p-6 overflow-x-hidden" style={{ paddingBottom: 'calc(80px + env(safe-area-inset-bottom))' }}>
                        {viewer}
                    </div>
                </Suspense>
            </div>
            <CopyPathMenu menu={copyMenu} onClose={closeCopyMenu} />
            <ExportMenu menu={exportMenu} options={exportOptions} busy={exporting} onPick={handleExport} onClose={closeExportMenu} />
        </div>
    )
}
