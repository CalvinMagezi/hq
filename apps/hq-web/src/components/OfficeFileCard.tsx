import { useAssetUrl } from '~/lib/useAssetUrl'

export function OfficeFileCard({ path: filePath }: { path: string }) {
    const fileUrl = useAssetUrl(filePath)
    const filename = filePath.split('/').pop() ?? filePath
    const ext = filename.split('.').pop()?.toUpperCase() ?? 'FILE'

    const iconMap: Record<string, string> = { PPTX: '📊', DOC: '📝', XLS: '📊' }
    const colorMap: Record<string, string> = { PPTX: 'var(--accent-amber)', DOC: 'var(--accent-blue)', XLS: 'var(--accent-green)' }
    const accent = colorMap[ext] ?? 'var(--accent-amber)'

    return (
        <div className="flex items-center justify-center py-16">
            <div
                className="p-8 rounded-2xl flex flex-col items-center gap-5 max-w-sm w-full"
                style={{
                    background: 'rgba(255,255,255,0.02)',
                    border: '1px solid rgba(255,255,255,0.06)',
                    backdropFilter: 'blur(12px)',
                    WebkitBackdropFilter: 'blur(12px)',
                    boxShadow: '0 8px 32px rgba(0,0,0,0.3)',
                }}
            >
                {/* File icon */}
                <div
                    className="w-16 h-16 rounded-2xl flex items-center justify-center text-2xl"
                    style={{
                        background: `color-mix(in srgb, ${accent} 8%, transparent)`,
                        border: `1px solid color-mix(in srgb, ${accent} 15%, transparent)`,
                        boxShadow: `0 0 24px color-mix(in srgb, ${accent} 6%, transparent)`,
                    }}
                >
                    {iconMap[ext] ?? '📎'}
                </div>

                {/* File info */}
                <div className="text-center">
                    <p className="text-sm font-mono font-bold truncate max-w-[280px]" style={{ color: 'var(--text-primary)' }}>
                        {filename}
                    </p>
                    <p className="text-[10px] font-mono mt-1.5" style={{ color: 'var(--text-dim)' }}>
                        {ext} file — preview not available
                    </p>
                </div>

                {/* Download button */}
                <a
                    href={fileUrl ?? undefined}
                    download
                    className="px-6 py-2.5 rounded-xl text-sm font-mono font-bold transition-all hover:brightness-110"
                    style={{
                        background: `color-mix(in srgb, ${accent} 12%, transparent)`,
                        color: accent,
                        border: `1px solid color-mix(in srgb, ${accent} 20%, transparent)`,
                    }}
                >
                    Download {ext}
                </a>
            </div>
        </div>
    )
}
