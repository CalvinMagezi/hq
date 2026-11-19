import { HqHttpError, hqFetch, hqJson } from './hqAuth'
import { defaultExtension, extensionFromDisposition, type NoteExportFormat } from './noteExport'

export interface PinnedNote {
  title: string
  path: string
  preview: string
  tags: string[]
  updatedAt?: string
  /** The agent that pinned it. Such a pin stays out of HQ's context until the owner keeps it. */
  suggestedBy?: string | null
}

export interface NoteTreeNode {
  name: string
  path: string
  type: 'dir' | 'file'
  children?: NoteTreeNode[]
}

export interface RecentFile {
  path: string
  title: string
  preview: string
  tags: string[]
  mtime: string
  size: number
  type?: string
  status?: string
  domain?: string
  primaryAgent?: string
  secondaryAgents?: string[]
  createdBy?: string
  updatedBy?: string
  project?: string
  visibility?: string
  decisionNeeded?: boolean
}

export interface VaultSignals {
  recent: RecentFile[]
  work: RecentFile[]
  review: RecentFile[]
  activity: RecentFile[]
}

export interface SearchHit {
  notePath: string
  title: string
  notebook: string
  snippet: string
  tags: string[]
  relevance: number
  matchType: string
}

export interface DirEntry {
  name: string
  path: string
  isDir: boolean
}

const SEARCH_LIMIT = 30

/** URL for a raw vault file. With a web token set, load it through useAssetUrl. */
export function assetUrl(path: string): string {
  return `/api/vault-asset?path=${encodeURIComponent(path)}`
}

export async function getPinnedNotes(): Promise<{ notes: PinnedNote[] }> {
  const res = await hqJson<{ notes: Array<PinnedNote & { mtime?: string }> }>('/api/pinned')
  return { notes: res.notes.map((n) => ({ ...n, updatedAt: n.mtime })) }
}

export function getVaultSignals(): Promise<VaultSignals> {
  return hqJson('/api/vault-signals')
}

export async function getVaultRoot(): Promise<{ path: string }> {
  const res = await hqJson<{ vault_path: string }>('/api/vault-status')
  return { path: res.vault_path }
}

/** The address of a notes search; HQ applies `folder` before the limit. */
export function searchPath(query: string, folder?: string | null): string {
  const scope = folder ? `&folder=${encodeURIComponent(folder)}` : ''
  return `/api/search?q=${encodeURIComponent(query)}&limit=${SEARCH_LIMIT}${scope}`
}

export async function searchNotes(query: string, folder?: string | null): Promise<{ results: SearchHit[] }> {
  if (!query.trim()) return { results: [] }
  const res = await hqJson<{ results: Array<Record<string, any>> }>(searchPath(query, folder))
  return {
    results: (res.results ?? []).map((r) => ({
      notePath: r.note_path,
      title: r.title,
      notebook: r.notebook ?? '',
      snippet: r.snippet ?? '',
      tags: r.tags ?? [],
      relevance: r.relevance ?? 0,
      matchType: r.match_type ?? 'keyword',
    })),
  }
}

export function getNoteTree(root: string): Promise<{ tree: NoteTreeNode }> {
  return hqJson(`/api/tree?recursive=true&path=${encodeURIComponent(root)}`)
}

export interface NoteDetailResponse {
  content: string
  path: string
  isDir: boolean
  dirEntries?: DirEntry[]
  notFound?: boolean
  hash?: string
}

export async function fetchNoteForDrawer(notePath: string): Promise<NoteDetailResponse> {
  const res = await hqFetch(`/api/note?path=${encodeURIComponent(notePath)}`)
  if (res.status === 404) {
    return { content: '', path: notePath, isDir: false, notFound: true }
  }
  if (!res.ok) {
    throw new Error(`Failed to load note: ${res.status} ${res.statusText}`)
  }
  const body = await res.json()
  return {
    content: body.content ?? '',
    path: body.path ?? notePath,
    isDir: !!body.isDir,
    dirEntries: body.entries,
    notFound: false,
    hash: body.hash,
  }
}

export async function getNote(
  notePath: string,
): Promise<{ content: string; isDir: boolean; dirEntries?: DirEntry[]; hash?: string }> {
  const res = await hqFetch(`/api/note?path=${encodeURIComponent(notePath)}`)
  if (res.status === 404) return { content: '', isDir: false }
  // Offline or server error: throw so a saved copy of the note stays on screen.
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`)
  const body = await res.json()
  return { content: body.content ?? '', isDir: !!body.isDir, dirEntries: body.entries, hash: body.hash }
}

/**
 * The note rendered by the server in one of the formats in `NOTE_EXPORT_OPTIONS` (frontmatter and
 * wikilinks cleaned up, vault images included). `extension` is the one the server named, which is not
 * always the one asked for: a note with several tables exports as a zip when CSV is requested.
 * Throws HqHttpError; for PDF, status 503 means the server's older PDF engine is not installed.
 */
export async function fetchNoteExport(
  notePath: string,
  format: NoteExportFormat,
): Promise<{ blob: Blob; extension: string; kept?: string }> {
  const res = await hqFetch(
    `/api/note/export?path=${encodeURIComponent(notePath)}&format=${encodeURIComponent(format)}`,
  )
  if (!res.ok) {
    const data = await res.json().catch(() => ({}))
    throw new HqHttpError(data.error || `${res.status} ${res.statusText}`, res.status)
  }
  return {
    blob: await res.blob(),
    extension: extensionFromDisposition(res.headers.get('content-disposition'), defaultExtension(format)),
    kept: res.headers.get('x-export-note') ?? undefined,
  }
}

export function togglePinNote(path: string, pinned: boolean): Promise<{ success: boolean }> {
  return hqJson('/api/pin', 'POST', { path, pin: pinned })
}

/** A deleted note waiting in the Trash. */
export interface TrashedNote {
  trash_path: string
  original_path: string
  trashed_on: string
  days_left: number
}

/** What a move did, or with `dryRun` what it would do: the notes whose links follow the moved one. */
export interface MoveResult {
  from: string
  to: string
  referrers: string[]
  linksRewritten: number
}

/** Moves or renames a note; links to it in other notes are rewritten. Moving it back undoes it. */
export async function moveNote(from: string, to: string, dryRun: boolean): Promise<MoveResult> {
  const r = await hqJson<{ from: string; to: string; referrers?: string[]; links_rewritten?: number }>('/api/note/move', 'POST', { from, to, dry_run: dryRun })
  return { from: r.from, to: r.to, referrers: r.referrers ?? [], linksRewritten: r.links_rewritten ?? 0 }
}

/** Moves a folder of notes, or with `dryRun` lists what would move. The notes keep their names, so links keep working. */
export async function moveFolder(from: string, to: string, dryRun: boolean): Promise<{ from: string; to: string; notes: string[] }> {
  const r = await hqJson<{ from: string; to: string; notes?: string[] }>('/api/folder/move', 'POST', { from, to, dry_run: dryRun })
  return { from: r.from, to: r.to, notes: r.notes ?? [] }
}

/** Moves the note to the Trash; `trash_path` is what Undo restores from. */
export function deleteNote(path: string): Promise<{ success: boolean; trash_path: string }> {
  return hqJson(`/api/note?path=${encodeURIComponent(path)}`, 'DELETE')
}

export function listTrash(): Promise<{ items: TrashedNote[]; retention_days: number }> {
  return hqJson('/api/note/trash')
}

export function readTrashed(trashPath: string): Promise<{ content: string }> {
  return hqJson(`/api/note/trash/read?path=${encodeURIComponent(trashPath)}`)
}

export function restoreNote(trashPath: string): Promise<{ success: boolean; path: string }> {
  return hqJson('/api/note/trash/restore', 'POST', { trash_path: trashPath })
}

/** `clientId` makes a retried create land on the same note instead of a second one. */
export function createNote(title: string, content: string, folder?: string, clientId?: string): Promise<{ success: boolean; path: string }> {
  return hqJson('/api/note/create', 'POST', { title, content, folder, client_id: clientId })
}

/** Someone else saved the note since this page loaded it. */
export interface NoteConflict {
  changed: true
  by?: string
  at?: string
  theirsHash?: string
}

export type SaveResult = { saved: true; hash?: string } | ({ saved: false } & NoteConflict)

/** Saves against `base`, the hash this page loaded; a 409 comes back as a conflict, not a throw. */
export async function updateNote(path: string, content: string, base?: string): Promise<SaveResult> {
  const res = await hqFetch('/api/note', {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ path, content, base }),
  })
  if (res.status === 409) {
    const b = await res.json().catch(() => ({}))
    return { saved: false, changed: true, by: b.by, at: b.at, theirsHash: b.theirs_hash }
  }
  if (!res.ok) {
    const b = await res.json().catch(() => ({}))
    throw new HqHttpError(b.error || `${res.status} ${res.statusText}`, res.status)
  }
  const b = await res.json().catch(() => ({}))
  return { saved: true, hash: b.hash }
}

export function getFolderList(): Promise<{ folders: string[] }> {
  return hqJson('/api/vault/folders')
}

/** One record view of a note or an office file: its content, never its layout. */
export interface FilePreview {
  path: string
  name: string
  kind: 'sheet' | 'doc' | 'slides' | 'file'
  title: string
  markdown: string
  /** What the preview leaves out, one plain line each. */
  notes: string[]
  /** Set when the preview is not the file's own look. */
  banner: string | null
}

export function getPreview(path: string): Promise<FilePreview> {
  return hqJson(`/api/suite/preview?path=${encodeURIComponent(path)}`)
}

/** The formats an Export sheet offers for a note, decided by the server from the note's own kind. */
export function getExportFormats(notePath: string): Promise<{ kind: string; formats: string[] }> {
  return hqJson(`/api/suite/export-formats?path=${encodeURIComponent(notePath)}`)
}

export interface MadeExport {
  files: Array<{ path: string; name: string }>
  fidelity: string
}

/** Makes the file under Exports/ with the same code the agent uses; the line says what was kept. */
export function makeExport(notePath: string, format: string): Promise<MadeExport> {
  return hqJson('/api/suite/export', 'POST', { path: notePath, format })
}

/** Saves a vault file through the browser's download. */
export async function downloadVaultFile(path: string, name: string): Promise<void> {
  const res = await hqFetch(assetUrl(path))
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`)
  const url = URL.createObjectURL(await res.blob())
  const a = document.createElement('a')
  a.href = url
  a.download = name
  document.body.appendChild(a)
  a.click()
  a.remove()
  setTimeout(() => URL.revokeObjectURL(url), 10_000)
}
