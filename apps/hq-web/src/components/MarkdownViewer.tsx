import { useEffect, useState, useRef } from 'react'
import { marked } from 'marked'
import DOMPurify from 'dompurify'
import { SelectionToolbar } from './SelectionToolbar'
import { useHQStore } from '~/store/hqStore'
import { useVaultNoteStore } from '~/store/vaultNoteStore'
import { blockRemoteImages } from '~/lib/remoteImages'
import { parseChart, parseMermaidPie, renderChartHtml } from '~/lib/chart'

// Lazy-load shiki only when first needed (avoids bundling all grammars eagerly)
let highlighterPromise: Promise<any> | null = null
function getHighlighterLazy() {
    if (!highlighterPromise) {
        highlighterPromise = import('shiki').then(({ createHighlighter }) =>
            createHighlighter({
                themes: ['vitesse-dark'],
                langs: ['javascript', 'typescript', 'tsx', 'json', 'bash', 'yaml', 'markdown', 'python'],
            })
        )
    }
    return highlighterPromise
}

function escapeHtml(text: string) {
    return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

// Configure marked renderer once at module level (not per-component-render)
// A promise, not a flag: concurrent callers must all wait until the renderer is registered.
let markedConfigured: Promise<void> | null = null
function ensureMarkedConfigured() {
    markedConfigured ??= configureMarked()
    return markedConfigured
}

// Chart SVG is built from validated numbers and escaped labels, then spliced in after DOMPurify,
// so the sanitizer profile stays HTML-only for untrusted markdown.
const chartSvgBySlot = new Map<number, string>()
let nextChartSlot = 0

function chartSlot(html: string) {
    const id = nextChartSlot++
    chartSvgBySlot.set(id, html)
    return `<div data-chart-slot="${id}"></div>`
}

function fillChartSlots(sanitized: string) {
    return sanitized.replace(/<div data-chart-slot="(\d+)"><\/div>/g, (match, id) => {
        const html = chartSvgBySlot.get(Number(id))
        chartSvgBySlot.delete(Number(id))
        return html ?? match
    })
}

async function configureMarked() {
    // A failed chunk load (offline, or stale after a redeploy) still registers the renderers, unhighlighted.
    const highlighter = await getHighlighterLazy().catch((err) => {
        console.warn('MarkdownViewer: syntax highlighting unavailable', err)
        return null
    })
    marked.use({
        renderer: {
            code(token: any) {
                let langLabel = escapeHtml(token.lang || '')
                if (token.lang === 'chart') {
                    const chart = parseChart(token.text)
                    if (chart.ok) return chartSlot(renderChartHtml(chart.spec))
                    langLabel = escapeHtml(`chart (not rendered: ${chart.reason})`)
                }
                if (token.lang === 'mermaid') {
                    const pie = parseMermaidPie(token.text)
                    if (pie.ok) return chartSlot(renderChartHtml(pie.spec))
                }
                const escaped = JSON.stringify(token.text)
                let highlighted: string
                try {
                    if (!highlighter) throw new Error('no highlighter')
                    highlighted = highlighter.codeToHtml(token.text, { lang: token.lang || 'text', theme: 'vitesse-dark' })
                } catch {
                    highlighted = `<pre><code>${escapeHtml(token.text)}</code></pre>`
                }
                return `<div class="code-block-wrapper">${langLabel ? `<div class="code-block-header">${langLabel}</div>` : ''}<button class="code-copy-btn" data-copy-text="${encodeURIComponent(token.text)}">Copy</button>${highlighted}</div>`
            },
            // Inline tokens go back through the parser, so **bold** and `code` inside them render.
            heading(this: any, token: any) {
                return `<h${token.depth} class="md-heading md-h${token.depth}">${this.parser.parseInline(token.tokens)}</h${token.depth}>`
            },
            blockquote(this: any, token: any) {
                return `<blockquote class="md-blockquote">${this.parser.parse(token.tokens)}</blockquote>`
            },
            link(this: any, token: any) {
                return `<a href="${token.href}" class="md-link" target="_blank" rel="noopener noreferrer">${this.parser.parseInline(token.tokens)}</a>`
            },
            table(this: any, token: any) {
                const align = (cell: any) => cell.align ? ` align="${cell.align}"` : ''
                const headerCells = token.header.map((cell: any) =>
                    `<th class="md-th"${align(cell)}>${this.parser.parseInline(cell.tokens)}</th>`
                ).join('')
                const rows = token.rows.map((row: any, i: number) =>
                    `<tr class="${i % 2 === 0 ? 'md-tr-even' : 'md-tr-odd'}">${
                        row.map((cell: any) => `<td class="md-td"${align(cell)}>${this.parser.parseInline(cell.tokens)}</td>`).join('')
                    }</tr>`
                ).join('')
                return `<div class="md-table-wrap"><table class="md-table"><thead><tr>${headerCells}</tr></thead><tbody>${rows}</tbody></table></div>`
            }
        }
    })
}

/** `bare` drops the frontmatter panel and selection toolbar, for chat bubbles. */
export function MarkdownViewer({ content, activePath, zoom, bare }: { content: string; activePath?: string; zoom?: number; bare?: boolean }) {
    const [html, setHtml] = useState<string>('')
    const [frontmatter, setFrontmatter] = useState<Record<string, any>>({})
    const [metaOpen, setMetaOpen] = useState(false)
    const containerRef = useRef<HTMLDivElement>(null)
    const { setGlobalChatOpen, setChatDraft } = useHQStore()

    const handleSendSelection = (text: string) => {
        setChatDraft(text)
        setGlobalChatOpen(true)
    }

    useEffect(() => {
        let active = true

        async function processContent() {
            // 1. Extract frontmatter
            let md = content || ''
            let fm: Record<string, any> = {}

            // Chat replies have no frontmatter, and one may open with a horizontal rule.
            const fmMatch = bare ? null : md.match(/^\uFEFF?---\r?\n([\s\S]*?)\r?\n---\r?\n/)
            if (fmMatch) {
                md = md.slice(fmMatch[0].length)
                const lines = fmMatch[1].split('\n')
                lines.forEach(line => {
                    const colonIdx = line.indexOf(':')
                    if (colonIdx > 0) {
                        const key = line.slice(0, colonIdx).trim()
                        const val = line.slice(colonIdx + 1).trim()
                        fm[key] = val
                    }
                })
            }
            setFrontmatter(fm)

            // 2. Ensure marked renderer is configured (only runs once globally)
            await ensureMarkedConfigured()

            // 3. Process Wikilinks [[...]]
            md = md.replace(/\[\[([^\]]+)\]\]/g, (_match, p1) => {
                const parts = p1.split('|')
                const target = parts[0].trim()
                const label = (parts[1] || parts[0]).trim()
                return `<button type="button" class="md-wikilink text-emerald-400 hover:text-emerald-300 underline font-medium cursor-pointer inline-block text-left" data-vault-note="${escapeHtml(target)}" aria-label="Open note ${escapeHtml(label)}">${escapeHtml(label)}</button>`
            })

            const raw = await marked.parse(md)
            const sanitized = DOMPurify.sanitize(raw, { USE_PROFILES: { html: true }, ADD_ATTR: ['data-copy-text', 'data-vault-note', 'data-chart-slot'] })
            const result = fillChartSlots(blockRemoteImages(sanitized))
            if (active) setHtml(result)
        }

        processContent().catch((err) => {
            console.warn('MarkdownViewer: render failed, showing plain text', err)
            if (active) setHtml(`<div style="white-space: pre-wrap">${escapeHtml(content || '')}</div>`)
        })
        return () => { active = false }
    }, [content, bare])

    // Delegated handler for code-copy buttons and vault note links
    useEffect(() => {
        const container = containerRef.current
        if (!container) return
        const handler = (e: MouseEvent) => {
            const btn = (e.target as Element).closest<HTMLElement>('[data-copy-text]')
            if (btn) {
                const text = decodeURIComponent(btn.getAttribute('data-copy-text') ?? '')
                navigator.clipboard.writeText(text).then(() => {
                    btn.textContent = 'Copied!'
                    setTimeout(() => { btn.textContent = 'Copy' }, 1500)
                }).catch(() => {})
                return
            }

            const noteBtn = (e.target as Element).closest<HTMLElement>('[data-vault-note]')
            if (noteBtn) {
                e.preventDefault()
                const notePath = noteBtn.getAttribute('data-vault-note')
                if (notePath) {
                    useVaultNoteStore.getState().openNote(notePath, noteBtn)
                }
                return
            }

            const link = (e.target as Element).closest<HTMLAnchorElement>('a')
            if (link) {
                const href = link.getAttribute('href') || ''
                const isVaultLink =
                    href.startsWith('vault:') ||
                    href.startsWith('note:') ||
                    href.startsWith('/api/note') ||
                    (href.endsWith('.md') && !href.includes('://'))
                if (isVaultLink) {
                    e.preventDefault()
                    useVaultNoteStore.getState().openNote(href, link)
                }
            }
        }
        container.addEventListener('click', handler)
        return () => container.removeEventListener('click', handler)
    }, [html])

    return (
        <div className="markdown-viewer relative" key={activePath} ref={containerRef} style={zoom && zoom !== 1 ? { zoom: `${zoom}` } as React.CSSProperties : undefined}>
            {!bare && <SelectionToolbar containerRef={containerRef} onSendAction={handleSendSelection} />}
            {!bare && Object.keys(frontmatter).length > 0 && (
                <div className="mb-6 rounded" style={{ background: 'var(--bg-elevated)', border: '1px solid var(--border)' }}>
                    <button
                        onClick={() => setMetaOpen(!metaOpen)}
                        className="w-full text-left px-3 py-2 text-xs font-mono font-bold flex justify-between items-center"
                        style={{ color: 'var(--text-dim)', cursor: 'pointer' }}
                    >
                        <span>📄 METADATA</span>
                        <span>{metaOpen ? '▾' : '▸'}</span>
                    </button>

                    {metaOpen && (
                        <div className="px-3 pb-3 text-xs font-mono border-t" style={{ borderColor: 'var(--border)' }}>
                            <div className="grid grid-cols-1 gap-1 pt-2">
                                {Object.entries(frontmatter).map(([k, v]) => (
                                    <div key={k} className="flex">
                                        <span className="w-24 opacity-60" style={{ color: 'var(--accent-blue)' }}>{k}</span>
                                        <span className="truncate" style={{ color: 'var(--text-primary)' }}>{v as React.ReactNode}</span>
                                    </div>
                                ))}
                            </div>
                        </div>
                    )}
                </div>
            )}

            <div
                className="md-content"
                dangerouslySetInnerHTML={{ __html: html }}
            />
        </div>
    )
}
