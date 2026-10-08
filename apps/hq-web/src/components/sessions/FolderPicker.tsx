import { useEffect, useState } from 'react'
import { ArrowUp, ChevronRight, FolderPlus, Folder, Loader2 } from 'lucide-react'
import { workbenchApi, type DirListing, type HostWorkspace } from '~/lib/sessionsApi'
import { crumbs, explorerLine } from '~/lib/workbench'

const INPUT_CLASS =
  'min-w-0 h-11 sm:h-9 px-3 rounded-lg text-xs font-mono text-neutral-200 bg-black/30 border border-white/10 focus:outline-none focus:ring-1 focus:ring-emerald-400'

interface Props {
  host: string
  workspace: HostWorkspace
  /** The chosen folder; blank means the HQ folder. */
  value: string
  onChange: (path: string) => void
}

/** Browse and pick a folder inside the computer's HQ folder, or make a new one. */
export function FolderPicker({ host, workspace, value, onChange }: Props) {
  const [listing, setListing] = useState<DirListing | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [newName, setNewName] = useState('')
  const [creating, setCreating] = useState(false)

  useEffect(() => {
    let current = true
    setLoading(true)
    setError(null)
    workbenchApi
      .dirs(host, value)
      .then((l) => current && setListing(l))
      .catch((e) => current && setError(e instanceof Error ? e.message : 'Could not list the folders.'))
      .finally(() => current && setLoading(false))
    return () => {
      current = false
    }
  }, [host, value])

  const shown = listing?.path ?? value
  const trail = crumbs(shown, workspace.root)
  const explorer = explorerLine(workspace, shown)

  const create = async () => {
    const name = newName.trim()
    if (!name) return
    setCreating(true)
    setError(null)
    try {
      const made = await workbenchApi.createDir(host, { ...(shown ? { parent: shown } : {}), name })
      setNewName('')
      onChange(made.path)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not make that folder.')
    } finally {
      setCreating(false)
    }
  }

  return (
    <div className="space-y-2">
      <nav aria-label="Folder path" className="flex flex-wrap items-center gap-x-0.5 text-[11px] font-mono text-neutral-400">
        {trail.map((c, i) => (
          <span key={c.path || 'home'} className="flex items-center">
            {i > 0 && <ChevronRight className="w-3 h-3 text-neutral-600" />}
            <button type="button" onClick={() => onChange(c.path)} className="min-h-11 sm:min-h-8 px-1.5 hover:text-white">
              {c.label}
            </button>
          </span>
        ))}
      </nav>
      <p className="text-[11px] font-mono text-neutral-300 break-all">{shown || workspace.root}</p>
      {explorer && <p className="text-[11px] font-mono text-neutral-500 break-all">{explorer}</p>}
      <div className="rounded-lg border border-white/10 bg-black/20 max-h-44 overflow-y-auto overscroll-contain">
        {listing?.parent != null && (
          <button
            type="button"
            onClick={() => onChange(listing.parent ?? '')}
            className="w-full flex items-center gap-2 min-h-11 px-3 text-left text-xs font-mono text-neutral-300 hover:bg-white/5 border-b border-white/5"
          >
            <ArrowUp className="w-3.5 h-3.5 text-neutral-500" />
            Up
          </button>
        )}
        {loading && (
          <div role="status" className="flex items-center gap-2 px-3 py-3 text-xs font-mono text-neutral-500">
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
            Looking for folders
          </div>
        )}
        {!loading && listing?.dirs.length === 0 && <p className="px-3 py-3 text-xs font-mono text-neutral-500">No folders inside this one yet.</p>}
        {!loading &&
          listing?.dirs.map((d) => (
            <button
              key={d.path}
              type="button"
              onClick={() => onChange(d.path)}
              className="w-full flex items-center gap-2 min-h-11 px-3 text-left text-xs font-mono text-neutral-200 hover:bg-white/5 border-b border-white/5 last:border-b-0"
            >
              <Folder className="w-3.5 h-3.5 text-neutral-500 shrink-0" />
              <span className="truncate">{d.name}</span>
            </button>
          ))}
      </div>
      {listing?.truncated && <p className="text-[11px] font-mono text-neutral-500">Only the first folders are shown. Make a new folder or go deeper to find yours.</p>}
      <div className="flex items-center gap-2">
        <input
          value={newName}
          onChange={(e) => setNewName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault()
              void create()
            }
          }}
          placeholder="New folder name"
          aria-label="New folder name"
          className={`${INPUT_CLASS} flex-1`}
        />
        <button
          type="button"
          onClick={() => void create()}
          disabled={creating || !newName.trim()}
          className="flex items-center gap-1.5 h-11 sm:h-9 px-3 rounded-lg border border-white/10 text-xs font-mono text-neutral-200 hover:bg-white/10 disabled:opacity-40 disabled:hover:bg-transparent shrink-0"
        >
          {creating ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <FolderPlus className="w-3.5 h-3.5" />}
          Make folder
        </button>
      </div>
      {error && (
        <p role="alert" className="text-[11px] font-mono text-rose-400 break-words">
          {error}
        </p>
      )}
    </div>
  )
}
