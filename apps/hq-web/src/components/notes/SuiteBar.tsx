import { useEffect, useState } from 'react'
import { t } from '~/strings'
import {
  fetchSuiteFile,
  suiteAbandon,
  suiteConfirm,
  suiteShrink,
  suiteUndo,
  type SuiteFile,
} from '~/lib/suiteApi'
import '~/components/vault/notes.css'

interface SuiteCall {
  (): Promise<SuiteFile>
}

/**
 * One thin bar over a note made from an office file: make that file smaller, say yes or no to what hq
 * prepared, or undo. Phone twin: `ui/notes/SuiteBar.kt`. hq writes every sentence about the file.
 */
export function SuiteBar({ source, onChanged }: { source: string; onChanged: () => void }) {
  const [file, setFile] = useState<SuiteFile | null>(null)
  const [busy, setBusy] = useState(false)
  const [refusal, setRefusal] = useState<string | null>(null)

  useEffect(() => {
    let live = true
    fetchSuiteFile(source)
      .then((f) => live && setFile(f))
      .catch(() => live && setFile(null))
    return () => {
      live = false
    }
  }, [source])

  if (!file) return null

  const run = async (call: SuiteCall) => {
    if (busy) return
    setBusy(true)
    setRefusal(null)
    try {
      setFile(await call())
      onChanged()
    } catch (e) {
      setRefusal(e instanceof Error && e.message ? e.message : t('suite_bar_failed'))
    } finally {
      setBusy(false)
    }
  }

  const job = file.job ?? ''
  return (
    <div className="an-suite" role="status" aria-live="polite">
      <p>{refusal ?? file.headline}</p>
      {!refusal && file.actions.confirm && file.details.map((line) => <small key={line}>{line}</small>)}
      {!refusal && (
        <div className="an-suite-btns">
          {busy ? (
            <span>{t('suite_bar_working')}</span>
          ) : (
            <>
              {file.actions.shrink && <button onClick={() => run(() => suiteShrink(source))}>{t('suite_bar_make_smaller')}</button>}
              {file.actions.confirm && <button onClick={() => run(() => suiteConfirm(job))}>{t('suite_bar_move')}</button>}
              {file.actions.abandon && <button onClick={() => run(() => suiteAbandon(job))}>{t('suite_bar_not_now')}</button>}
              {file.actions.undo && <button onClick={() => run(() => suiteUndo(job))}>{t('suite_bar_undo')}</button>}
            </>
          )}
        </div>
      )}
    </div>
  )
}
