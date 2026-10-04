import { useState } from 'react'
import { useAssetUrl } from '~/lib/useAssetUrl'

export function ImageViewer({ path }: { path: string }) {
    const fileUrl = useAssetUrl(path)
    const [scale, setScale] = useState(1)

    return (
        <div className="flex flex-col items-center justify-center h-full w-full rounded-lg overflow-hidden border" style={{ background: 'var(--bg-elevated)', border: '1px solid var(--border)' }}>
            <div className="p-2 w-full flex justify-between flex-shrink-0 border-b" style={{ borderColor: 'var(--border)', background: 'var(--bg-surface)' }}>
                <span className="text-xs font-mono" style={{ color: 'var(--text-dim)' }}>{path.split('/').pop()}</span>
                <div className="flex gap-2">
                    <button
                        onClick={() => setScale(s => Math.max(0.2, s - 0.2))}
                        className="px-2 text-xs rounded transition-colors"
                        style={{ background: 'var(--bg-base)', border: '1px solid var(--border)' }}
                    >-</button>
                    <span className="text-xs font-mono w-12 text-center leading-relaxed">{(scale * 100).toFixed(0)}%</span>
                    <button
                        onClick={() => setScale(s => Math.min(5, s + 0.2))}
                        className="px-2 text-xs rounded transition-colors"
                        style={{ background: 'var(--bg-base)', border: '1px solid var(--border)' }}
                    >+</button>
                </div>
            </div>
            <div className="flex-1 overflow-auto w-full flex items-center justify-center p-4">
                <img
                    src={fileUrl ?? undefined}
                    style={{ transform: `scale(${scale})`, transition: 'transform 0.1s cubic-bezier(0.4, 0, 0.2, 1)' }}
                    className="max-w-none shadow-2xl origin-center"
                    alt={path.split('/').pop() || 'Vault image'}
                />
            </div>
        </div>
    )
}
