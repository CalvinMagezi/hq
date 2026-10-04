import { useState, useEffect } from 'react'
import { getSpreadsheetData } from '~/lib/vaultApi'
import { useAssetUrl } from '~/lib/useAssetUrl'
import type { SheetData } from '~/lib/vaultApi'

export function SpreadsheetViewer({ path: filePath }: { path: string }) {
    const fileUrl = useAssetUrl(filePath)
    const [sheets, setSheets] = useState<SheetData[]>([])
    const [truncated, setTruncated] = useState(false)
    const [loading, setLoading] = useState(true)
    const [activeSheet, setActiveSheet] = useState(0)

    useEffect(() => {
        setLoading(true)
        getSpreadsheetData(filePath).then((res) => {
            setSheets(res.sheets)
            setTruncated(res.truncated)
            setActiveSheet(0)
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
                                style={{ background: 'var(--accent-green)', animationDelay: `${i * 0.15}s` }}
                            />
                        ))}
                    </div>
                    <span className="text-[10px] font-mono" style={{ color: 'var(--text-dim)' }}>
                        Parsing spreadsheet...
                    </span>
                </div>
            </div>
        )
    }

    if (sheets.length === 0) {
        return (
            <div className="text-center py-12 text-sm font-mono" style={{ color: 'var(--text-dim)' }}>
                No data found in spreadsheet
            </div>
        )
    }

    const sheet = sheets[activeSheet]

    return (
        <div>
            {/* Toolbar */}
            <div
                className="flex items-center justify-between mb-3 rounded-xl px-3 py-2 flex-wrap gap-2"
                style={{ background: 'rgba(255,255,255,0.02)', border: '1px solid rgba(255,255,255,0.05)' }}
            >
                <div className="flex items-center gap-2">
                    <span
                        className="text-[10px] font-mono px-2 py-1 rounded-lg font-bold tracking-wider"
                        style={{ background: 'rgba(0,255,163,0.1)', color: 'var(--accent-green)' }}
                    >
                        XLSX
                    </span>
                    <span className="text-[10px] font-mono" style={{ color: 'var(--text-dim)' }}>
                        {sheet.data.length > 1 ? `${sheet.data.length - 1} rows` : 'Empty'}
                        {truncated && <span style={{ color: 'var(--accent-amber)' }}> (truncated)</span>}
                    </span>
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

            {/* Sheet tabs */}
            {sheets.length > 1 && (
                <div
                    className="flex rounded-xl p-0.5 mb-3 overflow-x-auto"
                    style={{ background: 'rgba(255,255,255,0.03)', border: '1px solid rgba(255,255,255,0.04)' }}
                >
                    {sheets.map((s, i) => (
                        <button
                            key={s.name}
                            onClick={() => setActiveSheet(i)}
                            className="px-3 py-1.5 text-[10px] tracking-wider font-mono font-bold transition-all rounded-lg whitespace-nowrap"
                            style={{
                                color: activeSheet === i ? 'var(--accent-green)' : 'var(--text-dim)',
                                background: activeSheet === i ? 'rgba(0,255,163,0.08)' : 'transparent',
                            }}
                        >
                            {s.name}
                        </button>
                    ))}
                </div>
            )}

            {/* Table — glass-wrapped */}
            <div
                className="md-table-wrap rounded-xl"
                style={{
                    overflowX: 'auto',
                    WebkitOverflowScrolling: 'touch',
                    background: 'rgba(255,255,255,0.01)',
                }}
            >
                <table className="md-table" style={{ minWidth: '100%' }}>
                    {sheet.data.length > 0 && (
                        <thead>
                            <tr>
                                <th className="md-th" style={{ width: '40px', textAlign: 'center', color: 'var(--text-dim)', opacity: 0.5 }}>#</th>
                                {sheet.data[0].map((cell, j) => (
                                    <th key={j} className="md-th">
                                        {cell || `Col ${j + 1}`}
                                    </th>
                                ))}
                            </tr>
                        </thead>
                    )}
                    <tbody>
                        {sheet.data.slice(1).map((row, i) => (
                            <tr key={i} className={i % 2 === 0 ? 'md-tr-even' : 'md-tr-odd'}>
                                <td className="md-td" style={{ textAlign: 'center', color: 'var(--text-dim)', opacity: 0.4, fontSize: '10px' }}>{i + 1}</td>
                                {row.map((cell, j) => (
                                    <td key={j} className="md-td">
                                        {String(cell)}
                                    </td>
                                ))}
                            </tr>
                        ))}
                    </tbody>
                </table>
            </div>

            {truncated && (
                <div className="text-center py-3">
                    <span
                        className="text-[10px] font-mono px-3 py-1 rounded-lg inline-block"
                        style={{ background: 'rgba(255,179,0,0.06)', color: 'var(--accent-amber)', border: '1px solid rgba(255,179,0,0.1)' }}
                    >
                        Showing first 500 rows — download file for complete data
                    </span>
                </div>
            )}
        </div>
    )
}
