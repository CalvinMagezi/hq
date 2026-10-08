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
      <ol className="steps">
        <li>Open the <a href="https://console.hetzner.com/projects" target="_blank" rel="noreferrer">Hetzner Cloud console</a> and create a project, or open an empty one you can delete later.</li>
        <li>Go to Security, then API tokens, then Generate API token. Choose <strong>Read &amp; Write</strong>, and copy the token (Hetzner shows it once).</li>
        <li>Paste it below.</li>
      </ol>
      <p className="muted">
        A token belongs to one project, so this wizard can only touch that project. The token is sent to this site over HTTPS to make the Hetzner calls for you. It is not stored, logged or kept after you close the tab. Delete it in Hetzner when you are done.
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
