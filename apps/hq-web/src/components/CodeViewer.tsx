import { useEffect, useState } from 'react'

// Lazy-load shiki via dynamic import to keep it out of the main bundle
let highlighterPromise: Promise<any> | null = null

function getHighlighter() {
    if (!highlighterPromise) {
        highlighterPromise = import('shiki').then(({ createHighlighter }) =>
            createHighlighter({
                themes: ['vitesse-dark'],
                langs: ['javascript', 'typescript', 'tsx', 'jsx', 'json', 'yaml', 'bash', 'go', 'rust', 'html', 'css', 'sql', 'python']
            })
        )
    }
    return highlighterPromise
}

export function CodeViewer({ content, path }: { content: string; path: string }) {
    const [html, setHtml] = useState<string>('')
    const [copied, setCopied] = useState(false)

    const isTruncated = content.includes('...[TRUNCATED: File exceeds 500KB]...')
    const cleanContent = content.replace('\n\n...[TRUNCATED: File exceeds 500KB]...', '')

    useEffect(() => {
        let active = true
        const ext = path.split('.').pop()?.toLowerCase() || 'text'

        // map extensions to shiki langs
        const langMap: Record<string, string> = {
            'ts': 'typescript',
            'tsx': 'tsx',
            'js': 'javascript',
            'jsx': 'jsx',
            'json': 'json',
            'yml': 'yaml',
            'yaml': 'yaml',
            'sh': 'bash',
            'py': 'python',
            'go': 'go',
            'rs': 'rust',
            'html': 'html',
            'css': 'css',
            'sql': 'sql',
            'md': 'markdown'
        }
        const lang = langMap[ext] || 'text'

        getHighlighter().then((highlighter) => {
            if (!active) return
            try {
                const result = highlighter.codeToHtml(cleanContent, { lang, theme: 'vitesse-dark' })
                setHtml(result)
            } catch (e) {
                setHtml(`<pre><code>${cleanContent.replace(/</g, '&lt;').replace(/>/g, '&gt;')}</code></pre>`)
            }
        })

        return () => { active = false }
    }, [cleanContent, path])

    const copyToClipboard = () => {
        navigator.clipboard.writeText(cleanContent)
        setCopied(true)
        setTimeout(() => setCopied(false), 2000)
    }

    return (
        <div className="flex flex-col h-full w-full rounded-lg overflow-hidden border" style={{ background: 'var(--surface-200)', border: '1px solid var(--hairline)' }}>
            <div className="flex justify-between items-center p-2 border-b flex-shrink-0" style={{ borderColor: 'var(--hairline)', background: 'var(--surface-100)' }}>
                <span className="text-xs font-mono" style={{ color: 'var(--ink-muted)' }}>
                    {path.split('/').pop()}
                </span>
                <button
                    onClick={copyToClipboard}
                    className="px-3 py-1 text-[10px] font-mono rounded uppercase tracking-wider transition-colors"
                    style={{
                        background: copied ? 'var(--surface-200)' : 'var(--surface-000)',
                        color: copied ? 'var(--text-green)' : 'var(--ink-muted)',
                        border: '1px solid var(--hairline)'
                    }}
                >
                    {copied ? 'Copied' : 'Copy'}
                </button>
            </div>

            {isTruncated && (
                <div className="px-3 py-1 text-[10px] text-center font-mono" style={{ background: 'var(--surface-100)', color: 'var(--warn)' }}>
                    File exceeds 500KB. Showing first 500KB only.
                </div>
            )}

            <div className="flex-1 overflow-auto bg-[#121212] p-4 text-xs font-mono leading-relaxed code-content">
                {html ? (
                    <div dangerouslySetInnerHTML={{ __html: html }} />
                ) : (
                    <div className="flex items-center justify-center h-full">
                        <span className="text-xs font-mono animate-pulse" style={{ color: 'var(--ink-muted)' }}>Loading snippet...</span>
                    </div>
                )}
            </div>
        </div>
    )
}
