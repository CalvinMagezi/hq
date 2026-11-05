import { t } from '~/strings'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { AlertTriangle, Check, Eye, EyeOff, Loader2 } from 'lucide-react'
import {
  KEY_ANSWER_STRING,
  SETUP_PROVIDERS,
  checkSetupKey,
  fetchSetupStatus,
  saveSetupKey,
  showsNoSignIn,
  type KeyCheck,
  type SetupProviderId,
} from '~/lib/setupApi'

export const Route = createFileRoute('/setup')({
  component: SetupPage,
})

/** Not seen as a shell place: one column, 560 wide at most. */
function Column({ children }: { children: React.ReactNode }) {
  return (
    <div className="h-full w-full overflow-y-auto">
      <div className="mx-auto my-8 w-[calc(100%-2rem)] max-w-[560px] flex flex-col gap-5">
        <h1 className="a-display">{t('setup_title')}</h1>
        {children}
      </div>
    </div>
  )
}

/** F5: a server with no web sign-in cannot take a key from a browser. */
export function SetupNoSignIn() {
  return (
    <Column>
      <h2 className="a-h2">{t('setup_no_signin_title')}</h2>
      <p className="a-body-sm a-muted">{t('setup_no_signin_line')}</p>
      <code className="a-mono a-card">{t('setup_no_signin_command')}</code>
      <p className="a-body-sm a-muted">{t('setup_no_signin_after')}</p>
    </Column>
  )
}

export interface SetupFormProps {
  provider: SetupProviderId
  apiKey: string
  checking: boolean
  saving: boolean
  check: KeyCheck | null
  saveError: boolean
  onProvider: (p: SetupProviderId) => void
  onKey: (k: string) => void
  onCheck: () => void
  onOpen: () => void
}

/** F1 to F4: Check key asks the service and shows its answer; Open HQ saves, checked or not. */
export function SetupForm(p: SetupFormProps) {
  const [shown, setShown] = useState(false)
  const info = SETUP_PROVIDERS.find((x) => x.id === p.provider) ?? SETUP_PROVIDERS[0]
  const busy = p.checking || p.saving
  const empty = p.apiKey.trim() === ''
  const submit = (e: FormEvent) => {
    e.preventDefault()
    if (!empty && !busy) p.onOpen()
  }
  return (
    <Column>
      <form onSubmit={submit} className="flex flex-col gap-5">
        <div className="flex flex-col gap-1">
          <h2 className="a-h2">{t('setup_heading')}</h2>
          <p className="a-body-sm a-muted">{t('setup_line')}</p>
        </div>

        <fieldset className="flex flex-col gap-2" disabled={busy}>
          <legend className="a-field-label mb-1">{t('setup_service')}</legend>
          <div role="radiogroup" aria-label={t('setup_service')} className="flex flex-col gap-2">
            {SETUP_PROVIDERS.map((x) => {
              const chosen = x.id === p.provider
              return (
                <button
                  key={x.id}
                  type="button"
                  role="radio"
                  aria-checked={chosen}
                  className="a-rcard"
                  style={chosen ? { borderColor: 'var(--ink)' } : undefined}
                  onClick={() => p.onProvider(x.id)}
                >
                  <i className="a-radio" aria-hidden="true" />
                  <span>
                    <b>{t(x.name)}</b>
                    <span>{t(x.line)}</span>
                  </span>
                </button>
              )
            })}
          </div>
        </fieldset>

        <div className="a-field">
          <label className="a-field-label" htmlFor="hq-setup-key">
            {t('setup_key')}
          </label>
          <div className="flex gap-2">
            <input
              id="hq-setup-key"
              className="a-input a-mono flex-1 min-w-0"
              type={shown ? 'text' : 'password'}
              autoComplete="off"
              spellCheck={false}
              placeholder={t('setup_key_placeholder')}
              value={p.apiKey}
              disabled={busy}
              onChange={(e) => p.onKey(e.target.value)}
            />
            <button
              type="button"
              className="a-ib"
              aria-label={shown ? t('setup_key_hide') : t('setup_key_show')}
              aria-pressed={shown}
              onClick={() => setShown(!shown)}
            >
              {shown ? <EyeOff className="a-ic" /> : <Eye className="a-ic" />}
            </button>
          </div>
          <span className="a-field-msg">{t(info.help)}</span>
          {p.check && !p.check.accepted && (
            <p role="alert" className="a-field-msg flex items-start gap-1.5" style={{ color: 'var(--warn)' }}>
              <AlertTriangle className="w-4 h-4 flex-shrink-0" aria-hidden="true" />
              {t(KEY_ANSWER_STRING[p.check.answer])}
            </p>
          )}
        </div>

        <div className="flex flex-col gap-2">
          <button type="button" className="a-btn secondary block" disabled={empty || busy} onClick={p.onCheck}>
            {p.checking && <Loader2 className="a-ic animate-spin" aria-hidden="true" />}
            {p.checking ? t('setup_checking') : t('setup_check')}
          </button>
          {p.check?.accepted && (
            <p role="status" className="a-body-sm flex items-center gap-1.5">
              <Check className="w-4 h-4" aria-hidden="true" />
              {t('setup_accepted')}
            </p>
          )}
          <p className="a-body-sm a-muted">{t('setup_continue_unchecked')}</p>
          <button type="submit" className="a-btn block" disabled={empty || busy}>
            {p.saving && <Loader2 className="a-ic animate-spin" aria-hidden="true" />}
            {p.saving ? t('setup_saving') : t('setup_open')}
          </button>
          {p.saveError && (
            <p role="alert" className="a-field-msg" style={{ color: 'var(--warn)' }}>
              {t('setup_save_failed')}
            </p>
          )}
        </div>

        <p className="a-body-sm a-muted">{t('setup_later')}</p>
      </form>
    </Column>
  )
}

function SetupPage() {
  const navigate = useNavigate()
  const [provider, setProvider] = useState<SetupProviderId>(SETUP_PROVIDERS[0].id)
  const [apiKey, setApiKey] = useState('')
  const [checking, setChecking] = useState(false)
  const [saving, setSaving] = useState(false)
  const [check, setCheck] = useState<KeyCheck | null>(null)
  const [saveError, setSaveError] = useState(false)
  const [noSignIn, setNoSignIn] = useState(false)

  // An effect, not useQuery, for the same prerender reason as the root's SetupRedirect.
  useEffect(() => {
    fetchSetupStatus()
      .then((status) => {
        if (showsNoSignIn(status)) setNoSignIn(true)
        else if (!status.needs_setup) void navigate({ to: '/chat', replace: true })
      })
      .catch(() => undefined)
  }, [navigate])

  if (noSignIn) return <SetupNoSignIn />

  const onCheck = async () => {
    setChecking(true)
    setCheck(null)
    setCheck(await checkSetupKey(provider, apiKey))
    setChecking(false)
  }
  const onOpen = async () => {
    setSaving(true)
    setSaveError(false)
    try {
      await saveSetupKey(provider, apiKey)
      await navigate({ to: '/chat' })
    } catch {
      setSaveError(true)
    } finally {
      setSaving(false)
    }
  }
  return (
    <SetupForm
      provider={provider}
      apiKey={apiKey}
      checking={checking}
      saving={saving}
      check={check}
      saveError={saveError}
      onProvider={(p) => {
        setProvider(p)
        setCheck(null)
      }}
      onKey={(k) => {
        setApiKey(k)
        setCheck(null)
      }}
      onCheck={() => void onCheck()}
      onOpen={() => void onOpen()}
    />
  )
}
