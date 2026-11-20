import { useEffect, useState } from 'react'
import {
  NO_PROFILE,
  listSandboxProfiles,
  type SandboxGrantProfile,
} from '~/lib/sandboxGrantsApi'
import { profileAllowLines } from '~/components/settings/AccessView'
import { t } from '~/strings'

export interface AccessProfilePickerViewProps {
  id?: string
  profiles: SandboxGrantProfile[]
  value: string
  disabled?: boolean
  status?: string | null
  error?: string | null
  onChange: (name: string) => void
}

/** Pure selector for 'Allow development access': lists profiles plus 'None', and shows exactly what the chosen profile allows below it at full width. */
export function AccessProfilePickerView({
  id = 'task-access-profile',
  profiles,
  value,
  disabled,
  status,
  error,
  onChange,
}: AccessProfilePickerViewProps) {
  const chosen = value !== NO_PROFILE ? profiles.find((p) => p.name === value) ?? null : null
  const lines = chosen ? profileAllowLines(chosen) : []

  return (
    <div className="a-field">
      <label htmlFor={id} className="a-field-label">
        {t('grants_allow_dev_access')}
      </label>
      <div className="flex flex-col gap-2">
        <select
          id={id}
          value={value}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
          className="a-input sm:w-48"
        >
          <option value={NO_PROFILE}>{t('grants_profile_none')}</option>
          {profiles.map((p) => (
            <option key={p.name} value={p.name}>
              {p.name}
            </option>
          ))}
        </select>
        {chosen && (
          <div className="flex flex-col gap-0.5 w-full min-w-0 a-body-sm a-muted" data-testid="profile-preview">
            {lines.length === 0 ? (
              <span>{t('grants_profile_empty')}</span>
            ) : (
              lines.map((line) => (
                <span key={line} className="a-mono break-words">
                  {line}
                </span>
              ))
            )}
          </div>
        )}
      </div>
      {status && (
        <p role="status" className="a-cap a-muted">
          {status}
        </p>
      )}
      {error && (
        <p role="alert" className="a-cap a-warn-t">
          {error}
        </p>
      )}
    </div>
  )
}

export interface AccessProfilePickerProps {
  id?: string
  value: string
  disabled?: boolean
  status?: string | null
  error?: string | null
  onChange: (name: string) => void
}

/** Fetches configured access profiles and renders the selector. Hidden when the server has no profiles route or none configured. */
export function AccessProfilePicker(props: AccessProfilePickerProps) {
  const [profiles, setProfiles] = useState<SandboxGrantProfile[] | null>(null)

  useEffect(() => {
    let live = true
    listSandboxProfiles()
      .then((res) => {
        if (live) setProfiles(res.profiles)
      })
      .catch(() => {
        if (live) setProfiles([])
      })
    return () => {
      live = false
    }
  }, [])

  if (!profiles || profiles.length === 0) return null
  return <AccessProfilePickerView {...props} profiles={profiles} />
}
