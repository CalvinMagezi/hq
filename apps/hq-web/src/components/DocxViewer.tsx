import { useState, useEffect } from 'react'
import DOMPurify from 'dompurify'
import { getDocxAsHtml } from '~/lib/vaultApi'
import { useAssetUrl } from '~/lib/useAssetUrl'

export function DocxViewer({ path: filePath }: { path: string }) {
    const fileUrl = useAssetUrl(filePath)
    const [html, setHtml] = useState<string | null>(null)
    const [loading, setLoading] = useState(true)
    const [messages, setMessages] = useState<string[]>([])

    useEffect(() => {
        setLoading(true)
        getDocxAsHtml(filePath).then((res) => {
            setHtml(res.html)
            setMessages(res.messages)
            setLoading(false)
        })
    }, [filePath])

    if (loading) {
        return (
            <div className="flex items-center justify-center py-20">
                <div className="flex flex-col items-center gap-3">
                    <div className="inline-flex gap-1">
                        {[0, 1, 2].map((i) => (
                            <div
                                key={i}
                                className="thinking-dot"
                                style={{ background: 'var(--accent-blue)', animationDelay: `${i * 0.15}s` }}
                            />
                        ))}
                    </div>
                    <span className="text-[10px] font-mono" style={{ color: 'var(--text-dim)' }}>
                        Converting document...
                    </span>
                </div>
            </div>
        )
    }

    return (
        <div>
            {/* Toolbar */}
            <div
                className="flex items-center justify-between mb-3 rounded-xl px-3 py-2"
                style={{ background: 'rgba(255,255,255,0.02)', border: '1px solid rgba(255,255,255,0.05)' }}
            >
                <div className="flex items-center gap-2">
                    <span
                        className="text-[10px] font-mono px-2 py-1 rounded-lg font-bold tracking-wider"
                        style={{ background: 'rgba(68,136,255,0.1)', color: 'var(--accent-blue)' }}
                    >
                        DOCX
                    </span>
                    {messages.length > 0 && (
                        <span className="text-[10px] font-mono" style={{ color: 'var(--accent-amber)' }}>
                            {messages.length} warning{messages.length > 1 ? 's' : ''}
                        </span>
                    )}
                </div>
                <a
                    href={fileUrl ?? undefined}
                    download
                    className="px-3 py-1.5 rounded-xl text-[10px] font-mono font-bold transition-all hover:border-white/15"
                    style={{ background: 'rgba(255,255,255,0.04)', color: 'var(--text-dim)', border: '1px solid rgba(255,255,255,0.08)' }}
                >
                    Download
                </a>
            </div>

            {/* Rendered content — glass-wrapped document area */}
            <div
                className="docx-viewer rounded-xl p-5 sm:p-8"
                style={{
                    background: 'rgba(255,255,255,0.02)',
                    border: '1px solid rgba(255,255,255,0.05)',
                    backdropFilter: 'blur(8px)',
                    WebkitBackdropFilter: 'blur(8px)',
                }}
            >
                <div
                    className="markdown-viewer"
                    dangerouslySetInnerHTML={{ __html: DOMPurify.sanitize(html ?? '') }}
                />
            </div>
        </div>
    )
}
