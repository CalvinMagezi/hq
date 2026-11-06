import { useEffect, useState, useRef } from 'react'
import { marked } from 'marked'
import DOMPurify from 'dompurify'
import { t } from '~/strings'
import { SelectionToolbar } from './SelectionToolbar'
import { Sheet } from './ui/Sheet'
import { toastNotice } from '~/lib/toast'
import { useHQStore } from '~/store/hqStore'
import { useVaultNoteStore } from '~/store/vaultNoteStore'
import { blockRemoteImages } from '~/lib/remoteImages'
import { parseChart, parseMermaidPie, renderChartHtml } from '~/lib/chart'
import { codespanHtml, vaultLinkTarget } from '~/lib/noteRefs'

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
                    // The words say why, and the data stays visible as written.
                    langLabel = escapeHtml(t('chat_chart_fail', { reason: chart.reason }))
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
            // A backticked vault path opens the note, the way agents most often name one in chat.
            codespan(token: any) {
                return codespanHtml(token.text)
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
                return tableBlock(token, (tokens: any) => this.parser.parseInline(tokens))
            }
        }
    })
}

/** A table as the design's block (AlignedChatBlocks): its size and Copy and Expand, then the table in its own sideways scroll. */
export function tableBlock(token: { header: any[]; rows: any[][] }, inline: (tokens: any) => string): string {
    const align = (cell: any) => (cell.align ? ` align="${cell.align}"` : '')
    const head = token.header.map((cell: any) => `<th scope="col"${align(cell)}>${inline(cell.tokens)}</th>`).join('')
    const rows = token.rows.map((row: any[]) => `<tr>${row.map((cell: any) => `<td${align(cell)}>${inline(cell.tokens)}</td>`).join('')}</tr>`).join('')
    const size = escapeHtml(t('chat_table_size', { rows: token.rows.length, cols: token.header.length }))
    const bar =
        `<div class="hq-blk-h"><span>${size}</span>` +
        `<button type="button" class="a-ib sm" data-table-copy aria-label="${escapeHtml(t('chat_table_copy'))}"><span class="hq-ic hq-ic-copy" aria-hidden="true"></span></button>` +
        `<button type="button" class="a-ib sm" data-table-expand aria-label="${escapeHtml(t('chat_table_expand'))}"><span class="hq-ic hq-ic-expand" aria-hidden="true"></span></button></div>`
    return `<div class="hq-blk">${bar}<div class="a-scroll-x"><table class="a-table"><thead><tr>${head}</tr></thead><tbody>${rows}</tbody></table></div></div>`
}

/** A table's cells as tab-separated lines, the way a spreadsheet pastes them. */
export function tableText(table: HTMLTableElement): string {
    return Array.from(table.rows)
        .map((row) => Array.from(row.cells).map((cell) => (cell.textContent ?? '').trim()).join('\t'))
        .join('\n')
}

/** `bare` drops the frontmatter panel and selection toolbar, for chat bubbles. */
export function MarkdownViewer({ content, activePath, zoom, bare }: { content: string; activePath?: string; zoom?: number; bare?: boolean }) {
    const [html, setHtml] = useState<string>('')
    const [frontmatter, setFrontmatter] = useState<Record<string, any>>({})
    const [metaOpen, setMetaOpen] = useState(false)
    // A table opened on its own, at the width the page allows.
    const [expanded, setExpanded] = useState<string | null>(null)
    const containerRef = useRef<HTMLDivElement>(null)
    const setChatDraft = useHQStore((s) => s.setChatDraft)

    // The shell opens Chat with the text as its draft (it owns the router).
    const handleSendSelection = (text: string) => {
        setChatDraft(text)
        window.dispatchEvent(new Event('hq:open-chat'))
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
                return `<button type="button" class="md-wikilink a-link" data-vault-note="${escapeHtml(target)}" aria-label="Open note ${escapeHtml(label)}">${escapeHtml(label)}</button>`
            })

            const raw = await marked.parse(md)
            const sanitized = DOMPurify.sanitize(raw, { USE_PROFILES: { html: true }, ADD_ATTR: ['data-copy-text', 'data-vault-note', 'data-chart-slot', 'data-table-copy', 'data-table-expand'] })
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

            const tableBtn = (e.target as Element).closest<HTMLElement>('[data-table-copy],[data-table-expand]')
            const table = tableBtn?.closest('.hq-blk')?.querySelector('table')
            if (tableBtn && table) {
                if (tableBtn.hasAttribute('data-table-expand')) setExpanded(table.outerHTML)
                else void navigator.clipboard.writeText(tableText(table)).then(() => toastNotice(t('chat_table_copied'))).catch(() => undefined)
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
                const target = vaultLinkTarget(link.getAttribute('href'))
                if (target) {
                    e.preventDefault()
                    useVaultNoteStore.getState().openNote(target, link)
                }
            }
        }
        // The inline-code note chip is not a native button, so Enter and Space click it.
        const keyHandler = (e: KeyboardEvent) => {
            if (e.key !== 'Enter' && e.key !== ' ') return
            const chip = (e.target as Element).closest<HTMLElement>('code[data-vault-note]')
            if (!chip) return
            e.preventDefault()
            chip.click()
        }
        container.addEventListener('click', handler)
        container.addEventListener('keydown', keyHandler)
        return () => {
            container.removeEventListener('click', handler)
            container.removeEventListener('keydown', keyHandler)
        }
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
            {expanded && (
                <Sheet title={t('chat_table_expand')} placement="center" onClose={() => setExpanded(null)}>
                    {/* Already sanitized: it is a table taken from this viewer's own rendered HTML. */}
                    <div className="a-scroll-x" dangerouslySetInnerHTML={{ __html: expanded }} />
                </Sheet>
            )}
        </div>
    )
}
