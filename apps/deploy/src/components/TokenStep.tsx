'use client'

import { useState, type FormEvent } from 'react'

export function TokenStep({ busy, error, onSubmit }: { busy: boolean; error: string | null; onSubmit: (token: string) => void }) {
  const [token, setToken] = useState('')
  const submit = (e: FormEvent) => {
    e.preventDefault()
    onSubmit(token.trim())
  }
  return (
    <form onSubmit={submit} className="card stack">
      <h2>Connect your Hetzner project</h2>
      <p className="muted">
        Create a project API token with read and write access in the Hetzner Cloud console (Security, API tokens). Each token belongs to one
        project, so this wizard can only touch that project. The token is sent to this site over HTTPS to make the Hetzner calls for you. It
        is not stored, logged or kept after you close the tab, and you can delete it in Hetzner when you are done.
      </p>
      <label>
        Hetzner API token
        <input type="password" autoComplete="off" spellCheck={false} value={token} onChange={(e) => setToken(e.target.value)} />
      </label>
      {error && <p role="alert" className="error">{error}</p>}
      <button type="submit" disabled={busy || token.trim() === ''}>{busy ? 'Checking' : 'Continue'}</button>
    </form>
  )
}
