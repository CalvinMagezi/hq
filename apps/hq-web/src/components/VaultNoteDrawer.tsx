import { useCallback, useEffect, useState } from 'react'
import { Link } from '@tanstack/react-router'
import { AlertCircle, Check, ChevronRight, Copy, FileText, Folder } from 'lucide-react'
import { useVaultNoteStore } from '~/store/vaultNoteStore'
import { useHQStore } from '~/store/hqStore'
import { useBackClosesLayer } from '~/lib/backLayer'
import { fetchNoteForDrawer, type NoteDetailResponse } from '~/lib/vaultApi'
import { folderTrail } from '~/lib/search'
import { useEdition } from '~/lib/edition'
import { MarkdownViewer } from './MarkdownViewer'
import { Sheet } from './ui/Sheet'
import { t } from '~/strings'

const COPIED_FLASH_MS = 1500

/** A note opened from a chat or a link (AlignedChatSheets S1): a side sheet from 768 px, a bottom sheet below. */
export function VaultNoteDrawer() {
  const { open, notePath, closeNote } = useVaultNoteStore()
  useBackClosesLayer(open, closeNote)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [data, setData] = useState<NoteDetailResponse | null>(null)
  const [copied, setCopied] = useState(false)
  const lite = useEdition() === 'lite'

  const load = useCallback((path: string) => {
    let active = true
    setLoading(true)
    setError(null)
    fetchNoteForDrawer(path)
      .then((res) => active && setData(res))
      .catch((err) => active && setError(err instanceof Error ? err.message : String(err)))
      .finally(() => active && setLoading(false))
    return () => {
      active = false
    }
  }, [])

  useEffect(() => {
    if (open && notePath) return load(notePath)
    setData(null)
    setError(null)
    setLoading(false)
  }, [open, notePath, load])

  if (!open || !notePath) return null

  const title = data?.path ? data.path.split('/').pop()?.replace(/\.md$/, '') || data.path : notePath.split('/').pop()?.replace(/\.md$/, '') || notePath
  // Where it lives, the way the board writes it: "Projects › Agent-HQ", without the Notebooks root.
  const trail = folderTrail(notePath.split('/').slice(0, -1).filter((part) => part !== 'Notebooks').join('/'))
  const isNote = !!data && !data.notFound && !data.isDir

  const copyPath = () => {
    void navigator.clipboard.writeText(notePath)
    setCopied(true)
    setTimeout(() => setCopied(false), COPIED_FLASH_MS)
  }
  // The note goes into the chat's draft as a note reference, as Search's "Ask HQ about this note" does.
  const askAbout = () => {
    useHQStore.getState().setChatDraft(`\`${notePath}\``)
    closeNote()
    window.dispatchEvent(new Event('hq:open-chat'))
  }

  const footer = isNote ? (
    <>
      {!lite && (
        <button type="button" className="a-btn secondary" onClick={askAbout}>
          {t('chat_ask_about_this')}
        </button>
      )}
      <Link to="/notes/$" params={{ _splat: notePath }} className="a-btn" onClick={closeNote}>
        {t('chat_open_in_notes')}
      </Link>
    </>
  ) : undefined

  return (
    <Sheet
      title={title}
      placement="side"
      onClose={closeNote}
      footer={footer}
      headActions={
        <button type="button" className="a-ib sm" onClick={copyPath} aria-label={t('note_drawer_copy_path')}>
          {copied ? <Check className="a-ic sm a-ok-t" aria-hidden="true" /> : <Copy className="a-ic sm" aria-hidden="true" />}
        </button>
      }
    >
      {trail && <p className="a-body-sm a-muted">{trail}</p>}

      {loading && (
        <p role="status" className="a-loading">
          <span className="a-spin" aria-hidden="true" />
          {t('note_drawer_loading')}
        </p>
      )}

      {!loading && error && (
        <div className="a-card warn" role="alert">
          <div className="a-card-head">
            <AlertCircle className="a-ic sm a-warn-t" aria-hidden="true" />
            <b>{t('note_drawer_failed')}</b>
          </div>
          <p className="a-body-sm a-muted">{error}</p>
          <div className="a-btns">
            <button type="button" className="a-btn secondary compact" onClick={() => load(notePath)}>
              {t('note_drawer_try_again')}
            </button>
          </div>
        </div>
      )}

      {!loading && !error && data?.notFound && (
        <div className="a-empty">
          <span className="a-empty-ic">
            <FileText className="a-ic" aria-hidden="true" />
          </span>
          <b>{t('note_drawer_gone_title')}</b>
          <p>{t('note_drawer_gone_body', { title })}</p>
          <button type="button" className="a-btn secondary" onClick={closeNote}>
            {t('note_drawer_gone_close')}
          </button>
        </div>
      )}

      {!loading && !error && data?.isDir && (
        <div>
          <p className="a-grp">{t('note_drawer_folder')}</p>
          {(data.dirEntries ?? []).length === 0 && <p className="a-body-sm a-muted">{t('note_drawer_folder_empty')}</p>}
          {data.dirEntries?.map((entry) => (
            <button key={entry.path} type="button" className="a-row" onClick={() => useVaultNoteStore.getState().openNote(entry.path)}>
              {entry.isDir ? <Folder className="a-ic sm" aria-hidden="true" /> : <FileText className="a-ic sm" aria-hidden="true" />}
              <span className="a-row-main">
                <b>{entry.name}</b>
              </span>
              <ChevronRight className="a-ic sm" aria-hidden="true" />
            </button>
          ))}
        </div>
      )}

      {!loading && !error && isNote && <MarkdownViewer content={data.content || t('note_drawer_empty')} bare={false} />}
    </Sheet>
  )
}
