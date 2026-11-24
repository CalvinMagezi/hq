import { t } from '~/strings'
import { toastNotice } from '~/lib/toast'
import type { KeptConflict, SaveState } from './useSuiteEdit'

/** One quiet line that says whether the last change reached HQ, and what became of edits made with no connection. */
export function SaveLine({ state, conflict, onDismiss }: { state: SaveState; conflict?: KeptConflict | null; onDismiss?: () => void }) {
  return (
    <div className="su-saveblock">
      <p className="a-cap a-muted su-save" role="status">
        {state === 'saving' ? t('suite_edit_saving') : state === 'saved' ? t('suite_edit_saved') : state === 'queued' ? t('suite_offline_queued') : ''}
      </p>
      {conflict && (
        <div role="alert" className="su-conflict">
          <p className="a-body-sm a-warn-t">{t('suite_offline_conflict', { name: conflict.path.split('/').pop()?.replace(/\.md$/, '') ?? conflict.path })}</p>
          <div className="su-step">
            <button
              type="button"
              className="a-btn quiet compact"
              onClick={() => {
                void navigator.clipboard?.writeText(conflict.text).then(
                  () => toastNotice(t('suite_copied')),
                  () => undefined,
                )
              }}
            >
              {t('suite_offline_copy')}
            </button>
            <button type="button" className="a-btn quiet compact" onClick={onDismiss}>
              {t('suite_offline_drop')}
            </button>
          </div>
        </div>
      )}
    </div>
  )
}
