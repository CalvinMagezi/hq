interface SlashCommandDef {
  name: string
  args?: string
  description: string
}

export const SLASH_COMMANDS: SlashCommandDef[] = [
  { name: '/clear', description: 'Archive this chat and start a new one' },
  { name: '/topic', args: '<name>', description: 'Archive this chat and start a new one with that name' },
]

export function isSlashInput(val: string): boolean {
  return val.trimStart().startsWith('/')
}

export function matchingSlashCommands(val: string): SlashCommandDef[] {
  const q = val.trimStart().toLowerCase()
  if (!q.startsWith('/')) return []
  const token = q.split(' ')[0]
  return SLASH_COMMANDS.filter((c) => c.name.toLowerCase().startsWith(token))
}

export function parseSlashCommand(input: string): { command: string; args: string | null } | null {
  const trimmed = input.trim()
  if (!trimmed.startsWith('/')) return null
  const spaceIdx = trimmed.indexOf(' ')
  if (spaceIdx === -1) {
    return { command: trimmed.toLowerCase(), args: null }
  }
  const command = trimmed.slice(0, spaceIdx).toLowerCase()
  const args = trimmed.slice(spaceIdx + 1).trim()
  return { command, args: args.length > 0 ? args : null }
}

interface Props {
  input: string
  onSelect: (cmdName: string) => void
}

export function SlashCommandPalette({ input, onSelect }: Props) {
  const matches = matchingSlashCommands(input)
  if (matches.length === 0) return null

  return (
    <div
      className="absolute bottom-full left-0 right-0 mb-2 rounded-2xl overflow-hidden shadow-2xl z-50 text-xs backdrop-blur-xl"
      style={{
        background: 'rgba(15, 15, 15, 0.95)',
        border: '1px solid rgba(255, 255, 255, 0.1)',
        boxShadow: '0 -10px 40px rgba(0,0,0,0.6)',
      }}
    >
      <div className="px-3 py-2 text-[11px] font-bold uppercase tracking-wider border-b border-white/5 opacity-60" style={{ color: 'var(--accent-green)' }}>
        Slash Commands
      </div>
      <div className="max-h-48 overflow-y-auto p-1.5 space-y-1">
        {matches.map((cmd, idx) => (
          <button
            key={cmd.name}
            type="button"
            onClick={() => onSelect(cmd.name)}
            className={`w-full flex items-center justify-between px-3 py-2 rounded-xl text-left transition-colors ${
              idx === 0 ? 'bg-emerald-500/15 border border-emerald-500/30' : 'hover:bg-white/5 border border-transparent'
            }`}
          >
            <div className="flex items-center gap-2">
              <span className="font-bold text-emerald-400">{cmd.name}</span>
              {cmd.args && <span className="opacity-40 text-[11px]">{cmd.args}</span>}
            </div>
            <span className="text-[11px] opacity-60 truncate max-w-[200px]" style={{ color: 'var(--text-dim)' }}>
              {cmd.description}
            </span>
          </button>
        ))}
      </div>
    </div>
  )
}
