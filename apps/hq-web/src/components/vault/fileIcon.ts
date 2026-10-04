/** A one-glyph icon for a vault file, by extension. */
export function fileIcon(name: string) {
    if (name.endsWith('.md')) return '📄'
    if (name.endsWith('.pdf')) return '📕'
    if (name.match(/\.(png|jpe?g|gif|webp|svg)$/i)) return '🖼'
    if (name.endsWith('.json')) return '{}'
    if (name.match(/\.(ts|tsx|js|jsx)$/)) return '⟨/⟩'
    if (name.match(/\.(sh|yaml|yml)$/)) return '⚙'
    if (name.endsWith('.docx')) return '📝'
    if (name.endsWith('.xlsx') || name.endsWith('.xls')) return '📊'
    if (name.endsWith('.pptx')) return '📽'
    if (name.match(/\.html?$/)) return '🌐'
    return '📄'
}
