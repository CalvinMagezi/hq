import { useEffect, useState } from 'react'
import { ArrowUp, ChevronRight, FolderPlus, Folder, Loader2 } from 'lucide-react'
import { workbenchApi, type DirListing, type HostWorkspace } from '~/lib/sessionsApi'
import { crumbs, explorerLine, plainPath } from '~/lib/workbench'
import { ErrorText } from './ErrorText'

const INPUT_CLASS =
  'a-input min-w-0'

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
    <div className="flex flex-col gap-2">
      <nav aria-label="Folder path" className="flex flex-wrap items-center gap-x-0.5 a-cap a-muted">
        {trail.map((c, i) => (
          <span key={c.path || 'home'} className="flex items-center">
            {i > 0 && <ChevronRight className="a-ic" aria-hidden="true" />}
            <button type="button" onClick={() => onChange(c.path)} className="min-h-11 px-1.5 a-link">
              {c.label}
            </button>
          </span>
        ))}
      </nav>
      <p className="a-cap break-all">{plainPath(shown || workspace.root)}</p>
      {explorer && <p className="a-cap a-muted break-all">{explorer}</p>}
      <div className="a-term max-h-44 overflow-y-auto overscroll-contain p-0">
        {listing?.parent != null && (
          <button
            type="button"
            onClick={() => onChange(listing.parent ?? '')}
            className="a-menu-item"
          >
            <ArrowUp className="a-ic" aria-hidden="true" />
            Up
          </button>
        )}
        {loading && (
          <div role="status" className="flex items-center gap-2 px-3 py-3 a-body-sm a-muted">
            <Loader2 className="a-ic animate-spin" aria-hidden="true" />
            Looking for folders
          </div>
        )}
        {!loading && listing?.dirs.length === 0 && <p className="px-3 py-3 a-body-sm a-muted">No folders inside this one yet.</p>}
        {!loading &&
          listing?.dirs.map((d) => (
            <button
              key={d.path}
              type="button"
              onClick={() => onChange(d.path)}
              className="a-menu-item"
            >
              <Folder className="a-ic shrink-0" aria-hidden="true" />
              <span className="truncate">{d.name}</span>
            </button>
          ))}
      </div>
      {listing?.truncated && <p className="a-cap a-muted">Only the first folders are shown. Make a new folder or go deeper to find yours.</p>}
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
          className="a-btn secondary shrink-0"
        >
          {creating ? <Loader2 className="a-ic animate-spin" aria-hidden="true" /> : <FolderPlus className="a-ic" aria-hidden="true" />}
          Make folder
        </button>
      </div>
      {error && <ErrorText>{error}</ErrorText>}
      <p className="a-cap a-muted">Agents started in the HQ folder itself get their own new folder, so projects stay separate.</p>
    </div>
  )
}
