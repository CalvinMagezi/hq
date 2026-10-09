import { HqHttpError, hqFetch, hqJson } from './hqAuth'

export interface PinnedNote {
  title: string
  path: string
  preview: string
  tags: string[]
  updatedAt?: string
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

export interface SheetData {
  name: string
  data: string[][]
}

const SEARCH_LIMIT = 30
const SHEET_ROW_CAP = 500

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

export async function searchNotes(query: string): Promise<{ results: SearchHit[] }> {
  if (!query.trim()) return { results: [] }
  const res = await hqJson<{ results: Array<Record<string, any>> }>(
    `/api/search?q=${encodeURIComponent(query)}&limit=${SEARCH_LIMIT}`,
  )
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
  }
}

export async function getNote(
  notePath: string,
): Promise<{ content: string; isDir: boolean; dirEntries?: DirEntry[] }> {
  const res = await hqFetch(`/api/note?path=${encodeURIComponent(notePath)}`)
  if (res.status === 404) return { content: '', isDir: false }
  // Offline or server error: throw so a saved copy of the note stays on screen.
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`)
  const body = await res.json()
  return { content: body.content ?? '', isDir: !!body.isDir, dirEntries: body.entries }
}

/**
 * The note rendered as a PDF by the server (frontmatter and wikilinks cleaned up, vault images
 * included). Throws HqHttpError; status 503 means the server has no PDF engine installed.
 */
export async function fetchNotePdf(notePath: string): Promise<Blob> {
  const res = await hqFetch(`/api/note/pdf?path=${encodeURIComponent(notePath)}`)
  if (!res.ok) {
    const data = await res.json().catch(() => ({}))
    throw new HqHttpError(data.error || `${res.status} ${res.statusText}`, res.status)
  }
  return res.blob()
}

export function togglePinNote(path: string, pinned: boolean): Promise<{ success: boolean }> {
  return hqJson('/api/pin', 'POST', { path, pin: pinned })
}

export function createNote(title: string, content: string, folder?: string): Promise<{ success: boolean; path: string }> {
  return hqJson('/api/note/create', 'POST', { title, content, folder })
}

export function updateNote(path: string, content: string): Promise<{ success: boolean }> {
  return hqJson('/api/note', 'PUT', { path, content })
}

export function getFolderList(): Promise<{ folders: string[] }> {
  return hqJson('/api/vault/folders')
}

async function fetchBytes(path: string): Promise<ArrayBuffer> {
  const res = await hqFetch(`/api/vault-asset?path=${encodeURIComponent(path)}`)
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`)
  return res.arrayBuffer()
}

export async function getDocxAsHtml(path: string): Promise<{ html: string; messages: string[] }> {
  try {
    const mammoth = await import('mammoth')
    const result = await mammoth.convertToHtml({ arrayBuffer: await fetchBytes(path) })
    return { html: result.value, messages: result.messages.map((m: any) => m.message) }
  } catch (err: any) {
    return { html: `<p>Failed to convert document: ${err.message}</p>`, messages: [] }
  }
}

export async function getSpreadsheetData(path: string): Promise<{ sheets: SheetData[]; truncated: boolean }> {
  try {
    const XLSX = await import('xlsx')
    const workbook = XLSX.read(await fetchBytes(path), { type: 'array' })
    let truncated = false
    const sheets = workbook.SheetNames.map((name: string) => {
      const rows: string[][] = XLSX.utils.sheet_to_json(workbook.Sheets[name], { header: 1, defval: '' })
      if (rows.length > SHEET_ROW_CAP) truncated = true
      return { name, data: rows.slice(0, SHEET_ROW_CAP) }
    })
    return { sheets, truncated }
  } catch {
    return { sheets: [], truncated: false }
  }
}
