import { Link } from '@tanstack/react-router'
import { FileQuestion } from 'lucide-react'
import type { NoteTreeNode } from '~/lib/vaultApi'
import { t } from '~/strings'
import { VAULT_NAME, nearestFolder, noteName } from './noteRows'
import './notes.css'

/**
 * A link to something that is not in the Vault any more. It says so in plain words and offers three
 * ways out: the nearest folder that still exists, a search for the name, and the Trash. The location
 * bar above it stays, with the crumbs that are still there.
 */
export function VaultMissing({ path, root }: { path: string; root: NoteTreeNode | undefined }) {
  const name = noteName(path)
  const folder = nearestFolder(root, path)
  const folderName = folder === '' ? VAULT_NAME : noteName(folder)
  const search = () => window.dispatchEvent(new CustomEvent('hq:open-search', { detail: { scope: 'notes', q: name } }))
  return (
    <div className="an-home">
      <div role="alert" className="a-empty vn-missing">
        <span className="a-empty-ic">
          <FileQuestion className="a-ic" aria-hidden="true" />
        </span>
        <h2 className="a-h3">{t('vault_missing_title')}</h2>
        <p className="a-body-sm a-muted">{t('vault_missing_body', { name })}</p>
        <div className="a-btns">
          {folder === '' ? (
            <Link to="/notes" className="a-btn">
              {t('vault_missing_go', { folder: folderName })}
            </Link>
          ) : (
            <Link to="/notes/$" params={{ _splat: folder }} className="a-btn">
              {t('vault_missing_go', { folder: folderName })}
            </Link>
          )}
          <button type="button" className="a-btn secondary" onClick={search}>
            {t('vault_missing_search', { name })}
          </button>
          <Link to="/notes/trash" className="a-btn secondary">
            {t('vault_missing_trash')}
          </Link>
        </div>
      </div>
    </div>
  )
}
