import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { lazy, Suspense, useState, useCallback, useMemo, useRef } from 'react'
import { marked } from 'marked'
import DOMPurify from 'dompurify'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { FileQuestion } from 'lucide-react'
import { fetchNoteExport, togglePinNote, type DirEntry, type NoteTreeNode } from '~/lib/vaultApi'
import { exportFileName, exportGate, exportKeys, exportOptionsFor, type NoteExportFormat } from '~/lib/noteExport'
import { t } from '~/strings'
import { Dialog, workingLabel } from '~/components/ui/Dialog'
import { toastNotice, useToasts } from '~/lib/toast'
import { isSharedComputer } from '~/lib/sharedComputer'
import { useInHub } from '~/lib/hubWindow'
import { useSharedConnection } from '~/lib/connection'
import { AboutNote } from '~/components/notes/AboutNote'
import { HqHttpError } from '~/lib/hqAuth'
import { noteQuery, noteTreeQuery, vaultKeys } from '~/lib/queries'
import { MarkdownViewer } from '~/components/MarkdownViewer'
import { ImageViewer } from '~/components/ImageViewer'
import { NoteEditor } from '~/components/NoteEditor'
import { useHQStore } from '~/store/hqStore'
import { CopyPathMenu, type CopyMenuState } from '~/components/CopyPathMenu'
import { ExportMenu, type ExportMenuState } from '~/components/ExportMenu'
import { usePersistedState } from '~/lib/usePersistedState'
import { NoteTasks } from '~/components/tasks/NoteTasks'
import { NEW_CHAT_KEY, restoreDraft } from '~/components/chat/draftStore'
import { FolderView, NoteBar, NoteLayout, NoteMeta, SiblingList, BackLink } from '~/components/vault/NoteViews'
import { countNotes, parentPath } from '~/components/vault/noteRows'
import '~/components/vault/notes.css'

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

export const Route = createFileRoute('/notes/$')({
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
            <div className="an-home">
                {note.isError ? (
                    <div role="alert" className="a-empty">
                        <span className="a-empty-ic"><FileQuestion className="a-ic" aria-hidden="true" /></span>
                        <p>{t('notes_load_failed')}</p>
                        <BackLink path={filePath} />
                    </div>
                ) : (
                    <div role="status" aria-label={t('notes_viewer_loading')}>
                        <span className="a-skel t" />
                        <span className="a-skel t" />
                        <span className="a-skel t" />
                    </div>
                )}
            </div>
        )
    }
    if (note.data.isDir) return <FolderRoute dirPath={filePath} entries={note.data.dirEntries ?? []} />
    return (
        <VaultFileView
            key={filePath}
            filePath={filePath}
            content={note.data.content}
            hash={note.data.hash}
        />
    )
}

function subfolderCounts(tree: NoteTreeNode | undefined): Record<string, number> {
    const counts: Record<string, number> = {}
    for (const child of tree?.children ?? []) if (child.type === 'dir') counts[child.path] = countNotes(child)
    return counts
}

function FolderRoute({ dirPath, entries }: { dirPath: string; entries: DirEntry[] }) {
    const tree = useQuery(noteTreeQuery(dirPath))
    return (
        <FolderView
            dirPath={dirPath}
            entries={entries}
            counts={subfolderCounts(tree.data?.tree)}
            onNewHere={() => window.dispatchEvent(new CustomEvent('hq:open-quick-note', { detail: { folder: dirPath } }))}
        />
    )
}

function VaultFileView({ filePath, content, hash }: {
    filePath: string
    content: string
    hash?: string
}) {
    const { setGlobalChatOpen, setChatDraft } = useHQStore()
    const queryClient = useQueryClient()
    const navigate = useNavigate()

    const ext = filePath.split('.').pop()?.toLowerCase() ?? ''
    const isMd = ext === 'md'
    const parent = parentPath(filePath)
    const siblings = useQuery({ ...noteTreeQuery(parent ?? ''), enabled: parent !== null })
    const siblingEntries: DirEntry[] = (siblings.data?.tree?.children ?? []).map((c) => ({ name: c.name, path: c.path, isDir: c.type === 'dir' }))

    // All hooks must be called unconditionally before any early returns
    const initialPinned = isMd && /^---[\s\S]*?^pinned:\s*true/m.test(content)
    const [isPinned, setIsPinned] = useState(initialPinned)
    const [pinning, setPinning] = useState(false)
    const [editing, setEditing] = useState(false)
    const [sideOpen, setSideOpen] = useState(false)
    const [exporting, setExporting] = useState<NoteExportFormat | null>(null)
    const inHub = useInHub()
    const connection = useSharedConnection()
    const [askShared, setAskShared] = useState<NoteExportFormat | null>(null)
    const [exportNote, setExportNote] = useState<string | null>(null)
    const leaveRef = useRef<((go: () => void) => void) | null>(null)
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

    const handleAskHq = () => {
        restoreDraft(NEW_CHAT_KEY, `\`${filePath}\``)
        void navigate({ to: '/chat', search: { compose: '1' } })
    }

    const handleConvertToTask = () => {
        setChatDraft(
            `Convert this vault note into a task: ${filePath}. Use task_create_from_note to place it in the right Space > Folder > List, creating a new Space only if nothing fits.`,
        )
        setGlobalChatOpen(true)
    }

    // Server-rendered export in any of the offered formats. Touch devices get the share sheet
    // (WhatsApp, email, Drive); desktops get a download.
    const startExport = (format: NoteExportFormat) => {
        setExportMenu(null)
        const gate = exportGate({
            inHub,
            away: connection !== 'connected' && connection !== 'ai_down',
            shared: isSharedComputer(),
        })
        if (gate === 'window') setExportNote(t('note_window_export'))
        else if (gate === 'away') setExportNote(t('note_export_away'))
        else if (gate === 'ask') setAskShared(format)
        else void handleExport(format)
    }

    const handleExport = async (format: NoteExportFormat) => {
        if (exporting) return
        setExportNote(null)
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
            toastNotice(t('note_export_done', { name }))
        } catch (err) {
            if (format === 'pdf' && err instanceof HqHttpError && err.status === 503) {
                // The server's older PDF engine is not installed: print from the browser instead.
                await handlePrintPDF()
            } else {
                useToasts.getState().push({
                    text: t('note_export_failed', { kind: t(exportKeys(format).kind) }),
                    kind: 'alert',
                    action: { label: t('note_export_try_again'), run: () => void handleExport(format) },
                })
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
            useToasts.getState().push({ text: t('note_export_failed', { kind: t('note_export_kind_pdf') }), kind: 'alert' })
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
                    base={hash}
                    leaveRef={leaveRef}
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

    const toggleEditing = () => (editing && leaveRef.current ? leaveRef.current(() => setEditing(false)) : setEditing(!editing))

    const body = (
        <Suspense fallback={<span role="status" className="a-skel t" aria-label={t('notes_viewer_loading')} />}>
            <div className="an-md">
                {isMd && <NoteMeta path={filePath} />}
                {viewer}
                {exportNote && <p role="status" className="a-body-sm a-muted">{exportNote}</p>}
                {isMd && !editing && <NoteTasks path={filePath} />}
            </div>
        </Suspense>
    )

    return (
        <>
            <NoteLayout
                filePath={filePath}
                list={parent !== null && siblings.data ? <SiblingList parent={parent} entries={siblingEntries} current={filePath} /> : undefined}
                side={isMd ? <AboutNote path={filePath} /> : undefined}
                sideOpen={sideOpen}
                onToggleSide={() => setSideOpen(!sideOpen)}
                onMore={(e) => {
                    const rect = e.currentTarget.getBoundingClientRect()
                    setCopyMenu({ relPath: filePath, x: rect.right - 300, y: rect.bottom + 6 })
                }}
                bar={
                    isMd ? (
                        <NoteBar
                            editing={editing}
                            pinned={isPinned}
                            pinning={pinning}
                            exporting={exporting !== null}
                            exportOpen={exportMenu !== null}
                            onEdit={toggleEditing}
                            onAsk={handleAskHq}
                            onTask={handleConvertToTask}
                            onPin={handleTogglePin}
                            onExport={(e) => {
                                const rect = e.currentTarget.getBoundingClientRect()
                                setExportMenu({ x: rect.left, y: rect.bottom + 6 })
                            }}
                        />
                    ) : undefined
                }
            >
                {body}
            </NoteLayout>
            <CopyPathMenu
                menu={copyMenu}
                onClose={closeCopyMenu}
                onLinked={(p) => queryClient.invalidateQueries({ queryKey: vaultKeys.note(p) })}
                zoom={isMd && !editing ? {
                    pct: Math.round(mdZoom * 100),
                    canSmaller: mdZoom > MD_ZOOM_MIN,
                    canLarger: mdZoom < MD_ZOOM_MAX,
                    modified: mdZoom !== 1,
                    onSmaller: () => setMdZoomPersisted(mdZoom - MD_ZOOM_STEP),
                    onLarger: () => setMdZoomPersisted(mdZoom + MD_ZOOM_STEP),
                    onReset: () => setMdZoomPersisted(1),
                } : undefined}
            />
            <ExportMenu menu={exportMenu} options={exportOptions} busy={exporting} title={t('note_export_title')} onPick={startExport} onClose={closeExportMenu} />
            {askShared && (
                <Dialog
                    kind="ask"
                    title={t('note_export_shared_title')}
                    body={t('note_export_shared_body')}
                    keepLabel={t('note_export_shared_keep')}
                    actLabel={t('note_export_shared_act', { kind: t(exportKeys(askShared).kind) })}
                    busyLabel={workingLabel()}
                    onKeep={() => setAskShared(null)}
                    onAct={() => {
                        const f = askShared
                        setAskShared(null)
                        void handleExport(f)
                    }}
                />
            )}
        </>
    )
}
